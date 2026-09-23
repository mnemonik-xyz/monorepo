//! Agent-native publishing pipeline (webapp-rethink Task 9 / Decision 5).
//!
//! A blog post IS a signed PUBLIC attestation (Decision 8): there is no
//! parallel post store. Both publish surfaces — the native MCP tool
//! `mnemonic_publish_post` and the W3C-Micropub-shaped `POST /blog` — funnel
//! through the single [`publish_post`] pipeline here so the two paths cannot
//! drift in signing, persistence, or validation semantics.
//!
//! Pipeline (reuses the Task-7 core API exactly):
//!   1. the caller sends `signed_post`: a COSE_Sign1 over a canonical-CBOR
//!      `POST_V1` artifact, signed with the CALLER's own Ed25519 key
//!      (`producer` = `did:sol:<caller>`, `author` = display name).
//!   2. verify signature, canonical encoding and schema; the COSE `kid` MUST
//!      equal the authenticated caller. The server NEVER signs a post on a
//!      user's behalf (non-custodial invariant, same as memory writes).
//!      Only the operator's own identity may send plain fields and have the
//!      server sign with its own key.
//!   3. `content_hash` = blake3 of the signed canonical payload.
//!   4. persist the attestation with `Visibility::Public` + `WriteMode::Local`
//!      (Decision 9 — a V1 post is a FREE `local` public write; it is NOT a
//!      `participate` on-chain write, so it never touches x402).
//!   5. `store.upsert_blog_post(...)` keyed on `slug` (PK — re-publishing the
//!      same title REPLACES the row).
//!
//! Auth is enforced by the callers (OAuth2/Ed25519 bearer path in
//! `oauth/mod.rs`), independent of x402. Anonymous publish is rejected before
//! reaching this module.

use mnemonic_core::codec::canonical::{from_canonical_cbor, to_canonical_cbor};
use mnemonic_core::codec::schema::{validate_artifact, POST_V1};
use mnemonic_core::codec::sign::{sign_artifact, verify_artifact};
use mnemonic_core::storage::{AttestationStore, BlogPost, Visibility, WriteMode};

use crate::mcp::McpState;

/// Max title length (characters). Generous for headlines; bounds the slug and
/// the on-chain-ready artifact.
pub const MAX_TITLE_LEN: usize = 200;
/// Max markdown body length (bytes). Well under the 1 MiB body-peek cap the
/// bearer-auth middleware applies, so an authed `POST /blog` body always
/// survives middleware buffering.
pub const MAX_BODY_LEN: usize = 100_000;
/// Max number of tags on a post.
pub const MAX_TAGS: usize = 20;
/// Max length of a single tag (characters).
pub const MAX_TAG_LEN: usize = 64;
/// Max length of the human-readable author/agent display name (characters).
pub const MAX_AUTHOR_LEN: usize = 200;
/// Max slug length (characters) — truncates a very long title's kebab form.
pub const MAX_SLUG_LEN: usize = 96;

/// Validated, normalised publish request. Both surfaces build this from their
/// own wire shape (MCP tool args / Micropub JSON or form) before calling
/// [`publish_post`].
#[derive(Debug, Clone)]
pub struct PublishInput {
    pub title: String,
    pub body_markdown: String,
    pub tags: Vec<String>,
    /// Optional human-readable agent/display name. Falls back to the caller's
    /// authenticated identity when absent.
    pub author: Option<String>,
    /// COSE_Sign1 bytes over the canonical-CBOR `POST_V1` artifact, signed by
    /// the caller's own key. Required for every caller except the operator's
    /// own identity. When present, the fields above are ignored: title, body,
    /// tags and author come from the signed payload.
    pub signed_post: Option<Vec<u8>>,
}

/// Error text for a publish without a client signature.
pub const SIGNED_POST_REQUIRED: &str = "publish requires `signed_post`: a COSE_Sign1 over the \
     canonical-CBOR POST_V1 artifact, signed with your own key. The server never signs on \
     your behalf.";

/// Failure modes of [`publish_post`]. Each caller maps these to its own
/// transport: the MCP tool to a `JsonRpcError`, `POST /blog` to an HTTP status.
#[derive(Debug)]
pub enum PublishError {
    /// Caller-fixable: missing/empty/oversized field, or a title that
    /// slugifies to nothing. Maps to HTTP 400 / JSON-RPC `-32602`.
    InvalidInput(String),
    /// Per-identity publish rate limit tripped. Maps to HTTP 429 /
    /// JSON-RPC `-32005`.
    RateLimited,
    /// Server-side failure (signing, embedding, or persistence). Maps to
    /// HTTP 500 / JSON-RPC `-32603`.
    Internal(String),
}

impl PublishError {
    /// Human-readable message for the wire.
    pub fn message(&self) -> String {
        match self {
            PublishError::InvalidInput(m) => m.clone(),
            PublishError::RateLimited => "publish rate limit exceeded; retry shortly".to_string(),
            PublishError::Internal(m) => m.clone(),
        }
    }

    /// JSON-RPC error code for the MCP tool surface.
    pub fn json_rpc_code(&self) -> i32 {
        match self {
            PublishError::InvalidInput(_) => -32602,
            PublishError::RateLimited => -32005,
            PublishError::Internal(_) => -32603,
        }
    }

    /// Map to the typed `JsonRpcError` returned by `mnemonic_publish_post`.
    pub fn to_json_rpc(&self) -> crate::mcp::JsonRpcError {
        crate::mcp::JsonRpcError::simple(self.json_rpc_code(), self.message())
    }
}

/// Convert a title into a URL-safe kebab-case slug: lowercase ASCII
/// alphanumerics, runs of any other character collapsed to a single `-`, no
/// leading/trailing dash. Non-ASCII characters are dropped (a title that is
/// entirely non-ASCII therefore yields an empty slug — the caller rejects
/// that as `InvalidInput`). Capped at [`MAX_SLUG_LEN`].
pub fn slugify(title: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut prev_dash = false;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !slug.is_empty() {
            // Collapse any run of separators/punctuation/non-ASCII into one
            // dash; suppress a leading dash by gating on `!slug.is_empty()`.
            slug.push('-');
            prev_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.chars().count() > MAX_SLUG_LEN {
        slug = slug.chars().take(MAX_SLUG_LEN).collect();
        while slug.ends_with('-') {
            slug.pop();
        }
    }
    slug
}

/// Normalise + validate the raw fields of a publish request. Pure (no I/O) so
/// it is unit-testable and shared by both surfaces' wire parsers.
fn validate_input(
    input: &PublishInput,
) -> Result<(String, String, Vec<String>, String), PublishError> {
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err(PublishError::InvalidInput("title is required".into()));
    }
    if title.chars().count() > MAX_TITLE_LEN {
        return Err(PublishError::InvalidInput(format!(
            "title exceeds {MAX_TITLE_LEN} characters"
        )));
    }

    let body = input.body_markdown.clone();
    if body.trim().is_empty() {
        return Err(PublishError::InvalidInput(
            "body_markdown is required".into(),
        ));
    }
    if body.len() > MAX_BODY_LEN {
        return Err(PublishError::InvalidInput(format!(
            "body_markdown exceeds {MAX_BODY_LEN} bytes"
        )));
    }

    if input.tags.len() > MAX_TAGS {
        return Err(PublishError::InvalidInput(format!(
            "too many tags (max {MAX_TAGS})"
        )));
    }
    let mut tags = Vec::with_capacity(input.tags.len());
    for t in &input.tags {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        if t.chars().count() > MAX_TAG_LEN {
            return Err(PublishError::InvalidInput(format!(
                "tag exceeds {MAX_TAG_LEN} characters"
            )));
        }
        tags.push(t.to_string());
    }

    let author = input
        .author
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(MAX_AUTHOR_LEN).collect::<String>());

    Ok((title, body, tags, author.unwrap_or_default()))
}

/// Publish a post through the shared pipeline. `identity` is the authenticated
/// caller's pubkey (rate-limit key + author fallback). Returns the created
/// [`BlogPost`] (the exact shape `GET /blog/:slug` serves) on success.
///
/// Storage-lock discipline (CLAUDE.md): the embedding is computed BEFORE the
/// `rusqlite` mutex is taken; the critical section that writes the attestation
/// row + the blog_posts row is await-free and drops the guard before return.
pub fn publish_post(
    state: &McpState,
    identity: &str,
    input: PublishInput,
) -> Result<BlogPost, PublishError> {
    // Per-identity abuse control (Decision 5 — V1). Keyed on the authenticated
    // caller pubkey, NOT the IP, so one agent cannot drown the blog regardless
    // of its source addresses. An allowlist/moderation layer is a tracked open
    // item, not required for V1.
    if state
        .publish_limiter
        .check_key(&identity.to_string())
        .is_err()
    {
        return Err(PublishError::RateLimited);
    }

    let operator = state.keypair.pubkey_base58();
    let (post, content_hash) = match input.signed_post.as_deref() {
        Some(cose) => verify_signed_post(cose, identity)?,
        // The operator's own identity is the only author the server may sign
        // for: the key IS the author's key.
        None if identity == operator => operator_signed_post(state, identity, &input)?,
        None => return Err(PublishError::InvalidInput(SIGNED_POST_REQUIRED.into())),
    };
    let SignedPost {
        attestation_id,
        title,
        slug,
        body,
        tags,
        author,
        published_at: now,
    } = post;
    let signer = identity.to_string();

    // Embed the body for recall (blog = the ledger filtered to posts). Done
    // BEFORE the store lock — embedding may be slow (ONNX) and the rusqlite
    // guard must never wrap slow work.
    let embedding = state.embedder.embed(&body);

    // Single await-free critical section: write the public attestation row,
    // then upsert the blog_posts projection. A V1 post is a free `local`
    // public write — synthetic `local:` tx ids, no Arweave/Solana, no x402.
    {
        let store = state
            .store
            .lock()
            .map_err(|e| PublishError::Internal(format!("store mutex poisoned: {e}")))?;
        // Slug is the post's primary key: only its current owner may replace
        // it, so one author cannot overwrite another author's post.
        match store.blog_post_owner(&slug) {
            Ok(Some(owner)) if owner != signer => {
                return Err(PublishError::InvalidInput(format!(
                    "slug '{slug}' is already used by another author; choose a different title"
                )));
            }
            Ok(_) => {}
            Err(e) => return Err(PublishError::Internal(format!("slug lookup failed: {e}"))),
        }
        // A client-chosen artifact_id must not replace someone else's row.
        match store.attestation_owner(&attestation_id) {
            Ok(Some(owner)) if owner != signer => {
                return Err(PublishError::InvalidInput(
                    "artifact_id is already used by another identity".into(),
                ));
            }
            Ok(_) => {}
            Err(e) => return Err(PublishError::Internal(format!("id lookup failed: {e}"))),
        }
        let local_sol = format!("local:{}", &content_hash[..content_hash.len().min(16)]);
        let local_ar = format!("local:{}", &attestation_id[..attestation_id.len().min(8)]);
        store
            .save_attestation(
                &attestation_id,
                &body,
                &content_hash,
                &tags,
                &local_sol,
                &local_ar,
                &signer,
                &signer, // owner = the author who signed the post
                &now,
                WriteMode::Local,
                Visibility::Public,
                &embedding,
            )
            .map_err(|e| PublishError::Internal(format!("save_attestation failed: {e}")))?;
        store
            .upsert_blog_post(
                &slug,
                &title,
                &body,
                &tags,
                &author,
                &attestation_id,
                &content_hash,
                &now,
            )
            .map_err(|e| PublishError::Internal(format!("upsert_blog_post failed: {e}")))?;
    }

    // ── Rebuild-hook (Task 13) ───────────────────────────────────────────────
    // On publish success fire an optional, best-effort, NON-BLOCKING ping to
    // `BLOG_REBUILD_HOOK` so the standalone webapp re-prerenders `/blog/:slug`
    // for SEO freshness. The created post is returned regardless of the hook's
    // outcome — the JoinHandle is intentionally dropped (fire-and-forget).
    fire_rebuild_hook(state.blog_rebuild_hook.as_deref(), &state.hosted_client);

    Ok(BlogPost {
        slug,
        title,
        body_markdown: body,
        tags,
        author,
        attestation_id,
        content_hash,
        published_at: now,
    })
}

/// Post fields taken from a verified, signed `POST_V1` artifact.
struct SignedPost {
    attestation_id: String,
    title: String,
    slug: String,
    body: String,
    tags: Vec<String>,
    author: String,
    published_at: String,
}

fn invalid(msg: impl Into<String>) -> PublishError {
    PublishError::InvalidInput(msg.into())
}

/// Verify a client-signed post. The COSE `kid` must be the authenticated
/// caller, the payload must be canonical CBOR of a valid `POST_V1`, and the
/// signed fields must pass the same limits as the plain-field path.
fn verify_signed_post(cose: &[u8], identity: &str) -> Result<(SignedPost, String), PublishError> {
    let v =
        verify_artifact(cose, None).map_err(|e| invalid(format!("invalid signed_post: {e}")))?;
    if !v.valid {
        return Err(invalid("signed_post signature is not valid"));
    }
    if v.signer != identity {
        return Err(invalid(format!(
            "signed_post signer {} does not match the authenticated identity {identity}",
            v.signer
        )));
    }
    let json = from_canonical_cbor(&v.payload)
        .map_err(|e| invalid(format!("signed_post payload is not CBOR: {e}")))?;
    validate_artifact(&json, &POST_V1)
        .map_err(|e| invalid(format!("signed_post is not a valid POST_V1: {e}")))?;
    let canonical = to_canonical_cbor(&json, &POST_V1)
        .map_err(|e| invalid(format!("signed_post payload: {e}")))?;
    if canonical != v.payload {
        return Err(invalid("signed_post payload is not canonical CBOR"));
    }
    let field = |k: &str| {
        json.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    if field("type") != "post" {
        return Err(invalid("signed_post type must be \"post\""));
    }
    if field("producer") != format!("did:sol:{identity}") {
        return Err(invalid(
            "signed_post producer must be did:sol:<your pubkey>",
        ));
    }
    let tags: Vec<String> = json
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let raw = PublishInput {
        title: field("title"),
        body_markdown: field("content"),
        tags: tags.clone(),
        author: json
            .get("author")
            .and_then(|x| x.as_str())
            .map(String::from),
        signed_post: None,
    };
    let (title, body, norm_tags, author) = validate_input(&raw)?;
    if title != raw.title || norm_tags != tags {
        return Err(invalid(
            "signed_post title and tags must be trimmed and non-empty",
        ));
    }
    let slug = field("slug");
    if slug.is_empty() || slug != slugify(&title) {
        return Err(invalid("signed_post slug must equal slugify(title)"));
    }
    let attestation_id = field("artifact_id");
    if attestation_id.is_empty() || attestation_id.len() > 64 {
        return Err(invalid("signed_post artifact_id must be 1-64 characters"));
    }
    let post = SignedPost {
        attestation_id,
        title,
        slug,
        body,
        tags,
        author: if author.is_empty() {
            identity.to_string()
        } else {
            author
        },
        published_at: field("published_at"),
    };
    Ok((post, v.content_hash))
}

/// Plain-field publish by the operator's own identity: the server signs with
/// its own key, which is the author's key in this case only.
fn operator_signed_post(
    state: &McpState,
    identity: &str,
    input: &PublishInput,
) -> Result<(SignedPost, String), PublishError> {
    let (title, body, tags, author_raw) = validate_input(input)?;
    let author = if author_raw.is_empty() {
        identity.to_string()
    } else {
        author_raw
    };
    let slug = slugify(&title);
    if slug.is_empty() {
        return Err(invalid(
            "title produces an empty slug (needs at least one ASCII letter or digit)",
        ));
    }
    let attestation_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    // The markdown body sits in the standard `content` slot so
    // `content_hash` commits to the rendered source exactly as a memory's
    // does (Decision 8).
    let artifact = serde_json::json!({
        "artifact_id": attestation_id,
        "type": "post",
        "schema_version": 1,
        "title": title,
        "slug": slug,
        "content": body,
        "author": author,
        "published_at": now,
        "tags": tags,
        "created_at": now,
        "producer": state.keypair.did_sol(),
    });
    validate_artifact(&artifact, &POST_V1)
        .map_err(|e| PublishError::Internal(format!("POST_V1 validation failed: {e}")))?;
    let operator_keypair = state
        .keypair
        .keypair()
        .map_err(|e| PublishError::Internal(format!("operator identity unavailable: {e:#}")))?;
    let signed = sign_artifact(&artifact, &POST_V1, operator_keypair)
        .map_err(|e| PublishError::Internal(format!("COSE signing failed: {e}")))?;
    let post = SignedPost {
        attestation_id,
        title,
        slug,
        body,
        tags,
        author,
        published_at: now,
    };
    Ok((post, signed.content_hash))
}

/// Per-request timeout for the rebuild ping. Short so a slow/hung deploy
/// webhook never ties up a spawned task for long; the publish response has
/// already been returned regardless.
const REBUILD_HOOK_TIMEOUT_SECS: u64 = 5;

/// POST the rebuild ping to `url`. Errors are swallowed: the hook is a
/// best-effort freshness signal, never part of the publish contract.
async fn send_rebuild_ping(client: reqwest::Client, url: String) {
    if let Err(e) = client
        .post(&url)
        .timeout(std::time::Duration::from_secs(REBUILD_HOOK_TIMEOUT_SECS))
        .send()
        .await
    {
        tracing::debug!("blog rebuild hook ping failed (ignored): {e}");
    }
}

/// True only for an absolute `http`/`https` URL.
///
/// The blog rebuild hook is documented as http(s)-only; this is the real
/// enforcement of that allowlist (closing the doc/code gap flagged in the T14
/// security audit, SEC-T14-04). Any other scheme (`file:`, `ftp:`, …), a
/// non-absolute value, or an unparseable string is rejected — so a misconfigured
/// operator env var can never turn the best-effort freshness ping into a request
/// against an unintended target.
pub fn hook_url_allowed(url: &str) -> bool {
    match url::Url::parse(url) {
        Ok(parsed) => matches!(parsed.scheme(), "http" | "https"),
        Err(_) => false,
    }
}

/// Fire the best-effort, non-blocking blog-rebuild ping.
///
/// No-op (returns `None`) when `hook_url` is `None`/empty/whitespace, when its
/// scheme is not http(s) (see [`hook_url_allowed`]), or when called outside a
/// Tokio runtime (keeps [`publish_post`] callable from sync unit tests).
/// Otherwise spawns [`send_rebuild_ping`] on the current runtime and returns the
/// `JoinHandle` — production drops it (fire-and-forget); tests can await it to
/// observe delivery. SSRF posture: the shared `hosted_client` pins
/// `reqwest::redirect::Policy::none()`, so a deploy webhook cannot 302 the ping
/// to an unrelated host; the URL itself is operator-supplied via env, not
/// attacker input.
fn fire_rebuild_hook(
    hook_url: Option<&str>,
    client: &reqwest::Client,
) -> Option<tokio::task::JoinHandle<()>> {
    let url = hook_url?.trim();
    if url.is_empty() {
        return None;
    }
    // Defense in depth: enforce the http(s)-only allowlist at the firing point,
    // not only where the env var is parsed (main.rs). Any `McpState` built
    // directly — tests, future callers — still honors it (SEC-T14-04).
    if !hook_url_allowed(url) {
        tracing::warn!("ignoring blog rebuild hook — only absolute http(s) URLs are allowed");
        return None;
    }
    let handle = tokio::runtime::Handle::try_current().ok()?;
    Some(handle.spawn(send_rebuild_ping(client.clone(), url.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mnemonic_core::codec::sign::verify_artifact;
    use mnemonic_core::identity;
    use solana_sdk::signature::Keypair;

    #[test]
    fn slugify_basic_and_collapse_and_trim() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  Leading and trailing  "), "leading-and-trailing");
        assert_eq!(
            slugify("Multiple---separators__here"),
            "multiple-separators-here"
        );
        assert_eq!(slugify("Mnemonic v2: the Ledger"), "mnemonic-v2-the-ledger");
    }

    #[test]
    fn slugify_drops_non_ascii_to_empty() {
        // A wholly non-ASCII title yields an empty slug — `publish_post`
        // rejects that as InvalidInput.
        assert_eq!(slugify("日本語のタイトル"), "");
        assert_eq!(slugify("   "), "");
    }

    #[test]
    fn slugify_caps_length_without_trailing_dash() {
        let title = "a ".repeat(200); // many single-letter words → long kebab
        let slug = slugify(&title);
        assert!(slug.chars().count() <= MAX_SLUG_LEN);
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn post_v1_sign_verify_round_trip() {
        // Mirror the artifact `publish_post` builds and prove the POST_V1
        // COSE_Sign1 envelope verifies end-to-end against its content_hash.
        let kp = Keypair::new();
        let artifact = serde_json::json!({
            "artifact_id": "00000000-0000-0000-0000-000000000001",
            "type": "post",
            "schema_version": 1,
            "title": "Round Trip",
            "slug": "round-trip",
            "content": "# Body\n\nverifiable post.",
            "author": "Tester",
            "published_at": "2026-06-27T00:00:00+00:00",
            "tags": ["a", "b"],
            "created_at": "2026-06-27T00:00:00+00:00",
            "producer": identity::did_sol(&kp),
        });
        validate_artifact(&artifact, &POST_V1).expect("POST_V1 validates");
        let signed = sign_artifact(&artifact, &POST_V1, &kp).expect("sign");
        let result =
            verify_artifact(&signed.cose_bytes, Some(&signed.content_hash)).expect("verify");
        assert!(result.valid, "POST_V1 COSE must verify");
        assert_eq!(result.signer, identity::pubkey_base58(&kp));
    }

    // ── Rebuild-hook (Task 13) ───────────────────────────────────────────────

    #[test]
    fn hook_url_allowed_only_accepts_http_schemes() {
        // The documented http(s)-only allowlist is enforced in code (SEC-T14-04).
        assert!(hook_url_allowed("http://example.test/rebuild"));
        assert!(hook_url_allowed("https://example.test/rebuild"));
        // Non-http(s) schemes, non-absolute, and unparseable values are rejected.
        assert!(!hook_url_allowed("file:///etc/passwd"));
        assert!(!hook_url_allowed("ftp://example.test/x"));
        assert!(!hook_url_allowed("/relative/path"));
        assert!(!hook_url_allowed("not a url"));
    }

    #[tokio::test]
    async fn fire_rebuild_hook_rejects_non_http_scheme_even_in_runtime() {
        // A disallowed scheme is a no-op even with a Tokio runtime present —
        // proving the rejection comes from the scheme allowlist, not the
        // no-runtime guard (SEC-T14-04).
        let client = reqwest::Client::new();
        assert!(fire_rebuild_hook(Some("file:///etc/passwd"), &client).is_none());
        assert!(fire_rebuild_hook(Some("ftp://example.test/x"), &client).is_none());
    }

    #[test]
    fn fire_rebuild_hook_noop_when_unset_or_empty() {
        // Unset / empty / whitespace-only URL is a no-op regardless of runtime
        // (the empty-check short-circuits before any spawn).
        let client = reqwest::Client::new();
        assert!(fire_rebuild_hook(None, &client).is_none());
        assert!(fire_rebuild_hook(Some(""), &client).is_none());
        assert!(fire_rebuild_hook(Some("   "), &client).is_none());
    }

    #[test]
    fn fire_rebuild_hook_noop_outside_runtime() {
        // A set URL but no Tokio runtime in scope: must not panic, returns None
        // (keeps publish_post callable from plain sync unit tests).
        let client = reqwest::Client::new();
        assert!(fire_rebuild_hook(Some("http://127.0.0.1:9/hook"), &client).is_none());
    }

    #[tokio::test]
    async fn fire_rebuild_hook_pings_capture_server_when_set() {
        // Fires exactly one POST against a local capture server when the hook is
        // configured. Awaiting the returned handle proves the ping was spawned
        // (non-blocking) and completed.
        let server = httpmock::MockServer::start();
        let hook = server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/rebuild");
            then.status(200);
        });
        let client = reqwest::Client::new();
        let url = server.url("/rebuild");
        let handle = fire_rebuild_hook(Some(&url), &client).expect("spawns within runtime");
        handle.await.expect("ping task joins");
        hook.assert_calls(1);
    }

    #[tokio::test]
    async fn fire_rebuild_hook_best_effort_on_unreachable() {
        // An unreachable target: the spawned task swallows the send error and
        // still joins cleanly — the failure never propagates to the caller.
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(200))
            .build()
            .expect("client");
        let handle = fire_rebuild_hook(Some("http://127.0.0.1:9/nope"), &client)
            .expect("spawns within runtime");
        handle.await.expect("ping task joins despite send error");
    }

    // The hook-unset publish path (fire_rebuild_hook -> None, no-op, publish still
    // succeeds) is exercised end-to-end by the `POST /blog` integration tests in
    // tests/, whose `McpState` carries `blog_rebuild_hook: None` and flows through
    // `publish_post` — so it is not duplicated as a unit test here (publish.rs is
    // also compiled into the binary crate, which has no `test_support` module).
}
