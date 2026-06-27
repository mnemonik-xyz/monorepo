//! Agent-native publishing pipeline (webapp-rethink Task 9 / Decision 5).
//!
//! A blog post IS a signed PUBLIC attestation (Decision 8): there is no
//! parallel post store. Both publish surfaces — the native MCP tool
//! `mnemonic_publish_post` and the W3C-Micropub-shaped `POST /blog` — funnel
//! through the single [`publish_post`] pipeline here so the two paths cannot
//! drift in signing, persistence, or validation semantics.
//!
//! Pipeline (reuses the Task-7 core API exactly):
//!   1. build a `POST_V1` JSON artifact (markdown body in the standard
//!      `content` slot so `content_hash` commits to the rendered source);
//!      `producer` = the server's Ed25519 signer, `author` = the caller's
//!      agent/display name.
//!   2. `codec::schema::validate_artifact(&json, &POST_V1)`
//!   3. `codec::sign::sign_artifact(&json, &POST_V1, &keypair)`
//!   4. persist the attestation with `Visibility::Public` + `WriteMode::Local`
//!      (Decision 9 — a V1 post is a FREE `local` public write; it is NOT a
//!      `participate` on-chain write, so it never touches x402).
//!   5. `store.upsert_blog_post(...)` keyed on `slug` (PK — re-publishing the
//!      same title REPLACES the row).
//!
//! Auth is enforced by the callers (OAuth2/Ed25519 bearer path in
//! `oauth/mod.rs`), independent of x402. Anonymous publish is rejected before
//! reaching this module.

use mnemonic_core::codec::schema::{validate_artifact, POST_V1};
use mnemonic_core::codec::sign::sign_artifact;
use mnemonic_core::identity;
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
}

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

    let (title, body, tags, author_raw) = validate_input(&input)?;
    // Author display defaults to the authenticated identity when the caller
    // didn't supply one. `author` carries the agent name; `producer` (below)
    // is the cryptographic signer and stays distinct.
    let author = if author_raw.is_empty() {
        identity.to_string()
    } else {
        author_raw
    };

    let slug = slugify(&title);
    if slug.is_empty() {
        return Err(PublishError::InvalidInput(
            "title produces an empty slug (needs at least one ASCII letter or digit)".into(),
        ));
    }

    let attestation_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let signer = identity::pubkey_base58(&state.keypair);
    let producer = identity::did_sol(&state.keypair);

    // Build the POST_V1 artifact. The markdown body sits in the standard
    // `content` slot so `content_hash` commits to the rendered source exactly
    // as a memory's does (Decision 8).
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
        "producer": producer,
    });

    validate_artifact(&artifact, &POST_V1)
        .map_err(|e| PublishError::Internal(format!("POST_V1 validation failed: {e}")))?;

    let signed = sign_artifact(&artifact, &POST_V1, &state.keypair)
        .map_err(|e| PublishError::Internal(format!("COSE signing failed: {e}")))?;
    let content_hash = signed.content_hash;

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
                &signer, // owner = server signer: the post's publisher of record
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

/// Fire the best-effort, non-blocking blog-rebuild ping.
///
/// No-op (returns `None`) when `hook_url` is `None`/empty/whitespace, or when
/// called outside a Tokio runtime (keeps [`publish_post`] callable from sync
/// unit tests). Otherwise spawns [`send_rebuild_ping`] on the current runtime
/// and returns the `JoinHandle` — production drops it (fire-and-forget); tests
/// can await it to observe delivery. SSRF posture: the shared `hosted_client`
/// pins `reqwest::redirect::Policy::none()`, so a deploy webhook cannot 302 the
/// ping to an unrelated host; the URL itself is operator-supplied via env, not
/// attacker input.
fn fire_rebuild_hook(
    hook_url: Option<&str>,
    client: &reqwest::Client,
) -> Option<tokio::task::JoinHandle<()>> {
    let url = hook_url?.trim();
    if url.is_empty() {
        return None;
    }
    let handle = tokio::runtime::Handle::try_current().ok()?;
    Some(handle.spawn(send_rebuild_ping(client.clone(), url.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mnemonic_core::codec::sign::verify_artifact;
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
