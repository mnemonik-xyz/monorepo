# Code research: local-mode-survives-token-expiry

Terrain map for the user-spec. Cites file:line; no fixes proposed.

Repo conventions worth knowing up front:

- HTTP transport: `mcp/src/main.rs::run_http` wires `oauth::bearer_auth_middleware`
  in front of `/mcp` (`mcp/src/main.rs:850-860`). Stdio transport never touches
  this middleware (no JWT path).
- Storage discipline: `rusqlite::Connection` is `!Send`. Wrapped in `Mutex`;
  never held across `.await` (project CLAUDE.md hard rule).
- Architectural rule (CLAUDE.md): payment, pricing, OAuth, confirmation_token
  all live in `mcp/`, never in `core/`. The fix authors must respect this.

---

## A. HTTP auth middleware — where JWT validation happens before tool dispatch

### Where the middleware is wired

`mcp/src/main.rs:850-860` — `/mcp` and `/` (apex) both go through
`oauth::bearer_auth_middleware`:

```rust
let mcp_subrouter = Router::new()
    .route("/mcp", post(mcp::mcp_handler))
    .route("/", post(mcp::mcp_handler))
    .layer(middleware::from_fn_with_state(
        oauth_state.clone(),
        oauth::bearer_auth_middleware,
    ))
    .layer(GovernorLayer { config: mcp_governor_conf })
    .with_state(state.clone());
```

The middleware (and ONLY the middleware) decides whether the request reaches
`mcp::mcp_handler`. By the time `mcp_handler` runs, the JWT gate has already
fired.

### The middleware itself

`mcp/src/oauth/mod.rs:1382-1529` — `pub async fn bearer_auth_middleware`.
Shape:

1. URI allowlist (`mcp/src/oauth/mod.rs:1404-1437`) — `/oauth/*`, `/health`,
   `/.well-known/*`, `/api/pending/*`, `/api/sign-callback`,
   `/api/cli-bootstrap/redeem/*`, `/api/cli-bootstrap/redeem`,
   `/api/cli-bootstrap/issue-from-cli`, `/api/cli-bootstrap/server-pub`,
   `/api/extension-bootstrap/redeem/*`, `/api/key-escrow` — short-circuit
   `next.run(request).await` with no body inspection.

2. Buffer body (cap 1 MiB) at `mcp/src/oauth/mod.rs:1442-1451`.

3. Extract JSON-RPC `method` (`extract_json_rpc_method`,
   `mcp/src/oauth/mod.rs:1340-1343`) and, if it is `tools/call`, the tool name
   (`extract_tools_call_name`, `mcp/src/oauth/mod.rs:1351-1360`).

4. Build the `allowlisted` predicate (`mcp/src/oauth/mod.rs:1458-1480`):

```rust
let allowlisted = method
    .as_deref()
    .map(|m| {
        ALLOWLIST_METHODS.contains(&m)
            || m.starts_with("notifications/")
            || (m == "tools/call"
                && tools_call_name
                    .as_deref()
                    .map(|t| ALLOWLIST_TOOLS_CALL_NAMES.contains(&t))
                    .unwrap_or(false))
    })
    .unwrap_or(false);
```

5. Gated branch (`mcp/src/oauth/mod.rs:1496-1515`): JWT required AND must
   verify; on success Claims attached to extensions, body re-injected.
   Missing/invalid → `jsonrpc_unauthorized(StatusCode::UNAUTHORIZED, ...)`.

6. Allowlisted branch (`mcp/src/oauth/mod.rs:1517-1528`): JWT is OPTIONAL;
   if present and verifies, Claims attached; if absent/invalid, proceed
   without Claims — the comment is loud about this: "allowlisted requests
   must not 401 on bad tokens".

### The two allowlists today

`mcp/src/oauth/mod.rs:1326-1334`:

```rust
const ALLOWLIST_METHODS: &[&str] = &[
    "initialize",
    "tools/list",
    "ping",
    "prompts/list",
    "prompts/get",
    "resources/list",
    "resources/read",
];
```

`mcp/src/oauth/mod.rs:1362-1372`:

```rust
const ALLOWLIST_TOOLS_CALL_NAMES: &[&str] = &["mnemonic_recall"];
```

The comment at the per-tool allowlist is explicit (`mcp/src/oauth/mod.rs:1369-1371`):
"Adding a new entry expands the anonymous attack surface — review carefully
and pair with explicit visibility / tenancy guards in the downstream handler."

This is the place the spec's whitelist must live.

### What 401 looks like today

`mcp/src/oauth/mod.rs:1543-1570` — `fn jsonrpc_unauthorized`:

```rust
let body = serde_json::json!({
    "jsonrpc": "2.0",
    "id": Value::Null,
    "error": {"code": -32001, "message": format!("unauthorized: {msg}")}
});
let mut resp = (status, Json(body)).into_response();
if status == StatusCode::UNAUTHORIZED {
    let www_auth = format!(
        "Bearer realm=\"{issuer}\", error=\"invalid_token\", \
         error_description=\"{esc_msg}\", \
         resource_metadata=\"{issuer}/.well-known/oauth-protected-resource\"",
        ...
    );
    ...
}
```

Code is **`-32001`**, NOT `-32099`. "requires re-authorization (token expired)"
the user sees is the agent-side rendering of `verify_jwt` returning
`ExpiredSignature` → middleware emits `-32001 unauthorized: invalid JWT: ExpiredSignature`.

### `-32099 TokenExpired` — distinct path

`mcp/src/mcp.rs:446-456` defines the canonical error:

```rust
pub fn token_expired(expires_at: &str, pubkey: &str) -> JsonRpcError {
    JsonRpcError {
        code: -32099,
        message: "Token expired".to_string(),
        data: Some(serde_json::json!({
            "kind": "TokenExpired",
            "expires_at": expires_at,
            "pubkey": pubkey,
        })),
    }
}
```

But the doc above this function (`mcp/src/mcp.rs:431-445`) clarifies the
trigger: this is the typed error for the **soft-fall proxy** reading a
local cache (`~/.mnemonic/token.json`) via
`mnemonic_core::identity::token_store::read_token` and getting
`TokenStoreError::Expired`. The wire path lives in
`tools::proxy_participate` (`mcp/src/tools.rs:633-635`):

```rust
Err(mnemonic_core::identity::TokenStoreError::Expired { expires_at, sub }) => {
    return Err(ToolError::TypedRpc(token_expired(&expires_at, &sub)));
}
```

**Important:** `-32099 TokenExpired` is NOT what the HTTP middleware emits
when the inbound Bearer JWT is expired. Inbound expired JWT today emits
`-32001 unauthorized: invalid JWT: ExpiredSignature` (jsonwebtoken's
internal error string surfaces). The `-32099` typed error is reserved for
the soft-fall outbound flow.

### Origin of `jwt_sub`

`mcp/src/mcp.rs:1146-1152`:

```rust
let owner_pubkey: String = match &claims {
    Some(c) => c.sub.clone(),
    None => mnemonic_core::identity::pubkey_base58(&state.keypair),
};
let jwt_sub: Option<String> = claims.map(|c| c.sub);
```

`claims` is `Option<Claims>` pulled from request extensions earlier
(threaded through middleware). When the request is allowlisted AND no
Bearer was sent, both `claims` and `jwt_sub` are `None`. `owner_pubkey`
falls back to the SERVER keypair — see `mcp_handler` continues at
`mcp/src/mcp.rs:1154-1295` (paywall gate → handle_request_with_resolved_mode).
Threaded:

- `mcp/src/mcp.rs:1288-1295`: `handle_request_with_resolved_mode(&req, &state,
  &owner_pubkey, jwt_sub.as_deref(), resolved_mode_for_gate).await`
- which calls `handle_tool_call(name, &args, state, owner_pubkey, jwt_sub,
  pre_resolved_mode)` at `mcp/src/mcp.rs:974`

So if the middleware whitelist were extended to a `sign_memory` shape, the
handler would receive `jwt_sub = None` AND `owner_pubkey = server keypair
pubkey`. The downstream `sign_memory` already tolerates `jwt_sub = None`
(it's the stdio path — see §B).

### Confirmed: dispatcher rejects BEFORE `sign_memory` runs

The dispatcher does NOT have a per-tool short-circuit that lets a
"local+private" body slip past the gate. The gate is the middleware. The
fix is purely a middleware question.

### The public-counter precedent

The user-spec cites `mcp/src/api.rs:1239` as "Public, unauthenticated
counters". The actual route is registered at `mcp/src/main.rs:1108`:

```rust
.route("/stats", get(api::public_stats_handler))
```

— mounted OUTSIDE the `mcp_subrouter` (no `bearer_auth_middleware`
layer). Handler at `mcp/src/api.rs:1264-1301`. This is a different shape
from what the spec wants: a separate route, not a tool-call carve-out. The
relevant precedent for the spec is `ALLOWLIST_TOOLS_CALL_NAMES`
(`mcp/src/oauth/mod.rs:1372`), which already handles "tool-name carve-out
inside `tools/call`" — that's the shape to extend.

---

## B. sign_memory routing — what "explicit local" means in code

### The routing rule

`mcp/src/tools.rs:340-358`:

```rust
// Routing rule (round-2 simplification — security-auditor major):
// - `explicit local` (caller sent `mode: "local"`) → ALWAYS inline,
//   regardless of deploy. The user-spec invariant "Личная память
//   бесплатна всегда" is honoured uniformly: scenario (b) full + JWT
//   + explicit local AND scenario (c) local + JWT + explicit local
//   both produce a synthetic-id free local write. Closes the
//   round-1 gap where (c) silently went to the deferred path.
// - Everything else with a JWT → deferred (Cloud-tier flow). This
//   includes mode-absent + JWT on a local-only deploy (the
//   chrome-extension's actual production target — preserved byte-
//   for-byte) AND explicit `mode: "participate"` + JWT.
// - No JWT → inline (stdio / Claude Code path), unchanged.
if let Some(sub) = jwt_sub {
    if !resolved.is_explicit_local() {
        return sign_memory_deferred(embedder, compressor, pending, content, tags, sub)
            .await
            .map_err(ToolError::Other);
    }
}
```

`ResolvedMode::is_explicit_local()` at `mcp/src/tools.rs:73-75`:

```rust
pub fn is_explicit_local(&self) -> bool {
    self.explicit && self.write_mode == WriteMode::Local
}
```

So: even when `jwt_sub = None` (middleware whitelisted, no Claims), the
inline branch runs and writes a synthetic-id row. Good.

### Mode parsing (Option<&Value> → ResolvedMode)

`mcp/src/tools.rs:101-135` — `pub fn resolve_write_mode`:

```rust
pub fn resolve_write_mode(
    input_mode: Option<&serde_json::Value>,
    env_storage_mode: &str,
) -> Result<ResolvedMode, JsonRpcError> {
    match input_mode {
        None => {
            if env_storage_mode == "local" {
                Ok(ResolvedMode::fallback(WriteMode::Local))
            } else {
                Ok(ResolvedMode::fallback(WriteMode::Participate))
            }
        }
        Some(serde_json::Value::String(s)) => match WriteMode::from_str_strict(s) {
            Some(m) => Ok(ResolvedMode::explicit(m)),
            None => Err(invalid_params("mode", input_mode.expect("Some matched above"))),
        },
        Some(v) => Err(invalid_params("mode", v)),
    }
}
```

This is called UP FRONT in the dispatcher at `mcp/src/mcp.rs:1173-1195`
(BEFORE the paywall gate). For the spec's whitelist to work, the
middleware will need to recompute this from the buffered body (or accept
the simpler form: presence of literal `"mode":"local"` substring is NOT
safe; must parse JSON and check the field).

`ResolvedMode`: `mcp/src/tools.rs:44-76`. Two fields: `write_mode`
(`WriteMode::Local | Participate`) and `explicit: bool`. The
`is_explicit_local()` predicate is what carries the "user asked for local
explicitly" semantic the spec depends on.

### Visibility parsing

`mcp/src/tools.rs:155-178` — `pub fn resolve_visibility`:

```rust
pub fn resolve_visibility(
    args: &serde_json::Value,
    resolved_mode: WriteMode,
) -> Result<Visibility, JsonRpcError> {
    let raw = args.get("visibility");
    match raw {
        None => Ok(Visibility::default()),
        Some(v) => {
            if resolved_mode == WriteMode::Local {
                return Err(invalid_params("visibility", v));   // AC14
            }
            match v {
                serde_json::Value::String(s) => match Visibility::from_str_strict(s) {
                    Some(vis) => Ok(vis),
                    None => Err(invalid_params("visibility", v)),
                },
                _ => Err(invalid_params("visibility", v)),
            }
        }
    }
}
```

Key gotcha: **`mode: "local" + visibility` present (even `"private"`) is
already AC14 invalid params**. The user-spec's
`sign_memory{mode:"local", visibility:"private"}` payload would today
fail validation with `-32602`. So the spec's whitelist needs to either:
(a) accept just `mode:"local"` with NO `visibility` field (default is
`Visibility::Private` per `Visibility::default()`); OR (b) loosen the
visibility=private+local rule. The spec wording implies (a) — the
"local+private" framing is semantic, not literal-arg.

`Visibility::default()` lives in `core/src/storage/` — Private by
default; rows persist as `visibility='private'`.

### The public-write confirmation gate

`mcp/src/mcp.rs:1685-1742` — `request_public_write_confirmation` handler.
Belt-and-braces auth check at `mcp/src/mcp.rs:1700-1705`:

```rust
if jwt_sub.is_none() {
    return Err(JsonRpcError::simple(
        -32001,
        "request_public_write_confirmation requires authentication",
    ));
}
```

This is the gate the spec wants to KEEP intact for `participate+public`.
The whitelist for local+private MUST NOT include this tool name.

The confirmation_token ledger lives in `mcp/src/confirmation_token.rs`,
mint at `mcp/src/confirmation_token.rs:112-144`, returns
`(confirmation_token, jti, expires_at)`. TTL = 5 min
(`mcp/src/confirmation_token.rs:40`: `pub const DEFAULT_TTL: Duration =
Duration::from_secs(300)`).

### tools/list descriptor (for adding the new tool)

`mcp/src/mcp.rs:906-916` — `request_public_write_confirmation` advertise
block:

```rust
{
    "name": "request_public_write_confirmation",
    "description": "Public-write ceremony gate: ...",
    "inputSchema": {
        "type": "object",
        "properties": {
            "content_hash": {"type": "string", "description": "..."},
        },
        "required": ["content_hash"],
    },
},
```

This is the template shape `mnemonic_request_reauth` would slot into. There
is an existing assertion at `mcp/src/mcp.rs:1965` checking the tool count
("expected 7 MCP tools in tools/list response") — adding a tool will
require bumping this.

---

## C. recall — what does anonymous recall do today?

### Dispatcher branch

`mcp/src/mcp.rs:1640-1677` — `mnemonic_recall` handler:

```rust
"mnemonic_recall" => {
    let query = args["query"]
        .as_str()
        .ok_or_else(|| JsonRpcError::simple(-32603, "query required"))?;
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
    // Decision 5 / AC13 — agent-native-distribution (round 2 / SAR1-M1):
    //   - Anonymous caller (`jwt_sub.is_none()`): scope is the
    //     CROSS-OWNER public pool. Pass `owner_pubkey = None` AND
    //     `visibility_filter = Some(Public)`. The storage layer
    //     drops the owner predicate; only `visibility = 'public'`
    //     rows surface (private rows stay invisible regardless of
    //     owner — privacy contract preserved).
    //   - Authenticated caller: scope is the caller's own corpus
    //     across both visibilities. Pass `owner_pubkey = Some(sub)`
    //     AND `visibility_filter = None`.
    let (recall_owner, visibility_filter): (Option<&str>, _) = if jwt_sub.is_none() {
        (None, Some(mnemonic_core::storage::Visibility::Public))
    } else {
        (Some(owner_pubkey), None)
    };
    let store = state.store.lock().unwrap();
    tools::recall(
        &state.keypair,
        &store,
        state.embedder.as_ref(),
        query,
        limit,
        recall_owner,
        visibility_filter,
    )
}
```

### `args["owner_pubkey"]` is silently ignored

`grep` over `mcp.rs` confirms: the dispatcher reads only `query` and
`limit` from `args`. **The JSON-RPC `arguments.owner_pubkey` field is
discarded.** Recall's tenancy is entirely driven by `jwt_sub`. There is
no path where an anonymous caller can target a specific owner.

### Storage path

`mcp/src/tools.rs:1786-1831` — `pub fn recall`:

```rust
let results = store
    .search(&query_emb, owner_pubkey, visibility_filter, limit)
    .unwrap_or_default();
```

Doc comment at `mcp/src/tools.rs:1797-1812` confirms invariants. The
`AttestationStore::search` trait doc enforces "`None` owner must be paired
with `Some(visibility)`" — defence in depth.

### Anonymous recall today: works already

Per `mcp/src/oauth/mod.rs:1372`, `"mnemonic_recall"` is on
`ALLOWLIST_TOOLS_CALL_NAMES`. So today, `tools/call mnemonic_recall` with
no Bearer header is reachable and returns the cross-owner public pool.
Verified by integration test:

`mcp/tests/anonymous_recall.rs:78-117` — `filters_private_rows` test
seeds 1 private + 1 public, calls `call_tool(None, "mnemonic_recall",
...)`, asserts 200 + only public row.

So **AC2 of the user-spec is already satisfied today** — the spec author
should drop AC2 from the "fix" list and reframe it as a regression-anchor.
(See "Risks & gotchas" — there's a subtlety on token expiry impact.)

---

## D. request_public_write_confirmation — pattern for the new tool

### Response shape (template for `request_reauth`)

`mcp/src/mcp.rs:1732-1741`:

```rust
let (token, jti, expires_at) = state.confirmation_ledger.mint(
    content_hash,
    owner_pubkey,
    mnemonic_core::storage::Visibility::Public,
);
serde_json::json!({
    "confirmation_token": token,
    "jti": jti.to_string(),
    "expires_at": expires_at,
})
```

Returns 3 fields. `expires_at` is **unix seconds** (not a duration).
`confirmation_token.rs:119-123`:

```rust
let expires_at = (SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    + self.ttl)
    .as_secs();
```

The user-spec calls for `{authorize_url, expires_in}` — note `expires_in`
(duration) vs `expires_at` (timestamp). Either shape is workable; the
existing precedent leans toward `expires_at`.

### Auth requirement

NOT anonymous today. The middleware allowlist
(`ALLOWLIST_METHODS` / `ALLOWLIST_TOOLS_CALL_NAMES`) does NOT contain
`request_public_write_confirmation`, so it requires Bearer JWT to even
reach the handler. The handler has its own belt-and-braces check
(`mcp/src/mcp.rs:1700-1705`) returning `-32001` if `jwt_sub.is_none()`.

For `mnemonic_request_reauth` the spec explicitly says **anonymous**
(no auth required). That means it joins `ALLOWLIST_TOOLS_CALL_NAMES`
alongside `mnemonic_recall`.

### TTL

5 min (`mcp/src/confirmation_token.rs:40` — `DEFAULT_TTL`).
Background eviction tick: 60s (`mcp/src/confirmation_token.rs:43` —
`DEFAULT_EVICT_TICK`).

### Where the OAuth authorize URL comes from today

Two distinct flows in this codebase:

**1. Solana-wallet OAuth** (`mcp/src/oauth/mod.rs:486-...` —
`authorize_init_handler`). This is a server-side bootstrap that mints a
challenge and either returns JSON or 302-redirects to the webapp consent
page. There is no single "build authorize URL" helper — the URL the
external client constructs is `https://<host>/oauth/authorize?...` and
includes `client_id`, `redirect_uri`, `code_challenge`,
`code_challenge_method`, `state` query params (per
`AuthorizeInitQuery` at `mcp/src/oauth/mod.rs:434-440`).

**2. Google OAuth** (`mcp/src/oauth/google.rs:452-502` —
`google_start_handler`). Here the URL IS built server-side and the
client is 302-redirected:

```rust
let google_url = format!(
    "{auth}?{params}",
    auth = google.auth_url,
    params = build_query(&[
        ("client_id", &google.client_id),
        ("redirect_uri", &google.redirect_uri),
        ("response_type", "code"),
        ("scope", GOOGLE_SCOPES),
        ("state", &server_state),
        ("access_type", "online"),
        ("prompt", "select_account"),
    ])
);
redirect_to(&google_url)
```

The path the server exposes for the Google flow is
`/oauth/google/start` (mounted unauthenticated at
`mcp/src/main.rs:992-993`). Constants:

- `mcp/src/oauth/google.rs:110` — `DEFAULT_AUTH_URL =
  "https://accounts.google.com/o/oauth2/v2/auth"`
- `mcp/src/oauth/google.rs:78` — `GOOGLE_STATE_TTL_SECS: u64 = 600` (10
  minutes)

For `mnemonic_request_reauth` the natural source-of-truth is whatever
the deployment uses for the user's first-time OAuth — for HTTP MCP hosts
talking to `mcp.mnemonik.xyz`, that's the same `/oauth/authorize` (or
`/oauth/google/start`) URL the client used originally. There is **no
existing helper** that returns a ready-to-show URL string; the tool will
either need to build one or rely on a config field.

### Existing `re-auth` query-param distinction

None found. Grep over `mcp/src/` returns no usage of `re-auth=1` or
similar marker. The OAuth surface today does NOT distinguish first-time
vs renewal flows on the URL.

---

## E. Test harness — how to write integration tests for HTTP transport

### `mcp/src/test_support.rs` exposes

- `pub fn mock_state() -> Arc<McpState>` (`mcp/src/test_support.rs:95-162`)
  — `STORAGE_MODE=local`, `PAYMENT_MODE=none`, stub embedder, in-memory
  SQLite per call.
- `pub fn mock_state_with(storage_mode, payment_mode, cost)` (`:172-242`)
  — parameterised variant.
- `pub fn mock_state_for_delivery(...)` (`:258-331`) — soft-fall /
  delivery tests.
- `pub fn mock_state_with_embedder_and_endpoint(...)` (`:349-417`) —
  injects custom embedder + hosted endpoint.
- `pub fn mint_jwt(sub: &str, secret: &[u8]) -> String`
  (`mcp/src/test_support.rs:441-457`) — issues a `iss=mcp.mnemonik.xyz`,
  `aud=mcp`, `iat=now`, `exp=now+3600` token. The `exp` is hardcoded at
  `now + 3600` (line 448) — for testing expiry, the spec authors will
  need either a new helper that takes `exp` explicitly OR a wrapper that
  mints with negative `exp` (the underlying `encode` allows it).

`#[cfg(any(test, feature = "test-support"))]`-gated (declared at
`mcp/src/lib.rs`). Tests that use it must declare `#![cfg(feature =
"test-support")]` at file top (see `mcp/tests/anonymous_recall.rs:25`).

### `TestServer` harness

`mcp/tests/_helpers/mod.rs:131-296` — builds a router mirroring production
HTTP wiring (router, bearer-auth middleware, McpState). Key methods:

- `TestServer::call_tool(sub: Option<&str>, name, arguments)`
  (`mcp/tests/_helpers/mod.rs:184-212`) — sends `tools/call`; `sub =
  None` ⇒ no Authorization header.
- `TestServer::mint_jwt(sub)` — wraps `test_support::mint_jwt` with the
  shared test secret.
- `TestServer::with_token(jwt)` — bind a token to an `AuthedClient` for
  cross-call reuse.

The harness CAN spin up HTTP transport without JWT (just pass
`sub = None`) and with JWT (pass a sub). It does NOT yet have an
"artificially-expired JWT" helper. To get one:

- Either inline a copy of `mint_jwt` with `exp = now - 1`, OR
- Add a `mint_expired_jwt` helper to `test_support.rs`.

The JWT verify path (`oauth::verify_jwt`, `mcp/src/oauth/mod.rs:401-423`)
calls `Validation::new(Algorithm::HS256)` and sets `validate_exp = true`,
so an `exp` in the past surfaces as `JWT verify failed: ExpiredSignature`
and the middleware returns `-32001`. That maps cleanly to the user-spec's
"token expired" symptom.

### 401-without-JWT pattern

`mcp/src/mcp.rs:2151-2178` — `test_missing_authorization_header_returns_401`
is the canonical example:

```rust
#[tokio::test]
async fn test_missing_authorization_header_returns_401() {
    let state = build_test_state();
    let app = build_test_router(state);

    let req_body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "params": {"name": "mnemonic_sign_memory", "arguments": {"content": "x"}},
        "id": 99,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        // NO Authorization header — Task 4a will reject this.
        .body(Body::from(serde_json::to_vec(&req_body).expect("serialize req")))
        .expect("build request");

    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "Task 4a must reject /mcp tools/call without Bearer JWT",
    );
}
```

`mcp/tests/auth_allowlist.rs:64-136` —
`test_tools_list_initialize_no_auth_200_sign_memory_no_auth_401` — covers
both 200 (`tools/list`, `initialize`) and 401 (`tools/call sign_memory`)
in a single test using the `_helpers` harness shape.

The middleware unit-test pattern at `mcp/src/oauth/mod.rs:2697-2733`
(`test_middleware_allowlisted_request_without_jwt_passes_no_claims`)
is the minimal shape for a middleware-level regression — useful if the
spec wants both granular unit tests (middleware echoes claims) and
integration tests (full /mcp roundtrip).

---

## F. Token model — what does "expired" mean in our token?

### JWT structure

`mcp/src/oauth/mod.rs:380-397` — `issue_jwt_with_google_sub`:

```rust
pub fn issue_jwt_with_google_sub(
    state: &OAuthState,
    sub: &str,
    google_sub: Option<String>,
) -> Result<String, String> {
    let now = now_secs();
    let claims = Claims {
        iss: JWT_ISSUER.to_string(),
        aud: JWT_AUDIENCE.to_string(),
        sub: sub.to_string(),
        iat: now,
        exp: now + JWT_TTL_SECS,
        jti: uuid::Uuid::new_v4().to_string(),
        google_sub,
    };
    let header = Header::new(Algorithm::HS256);
    encode(&header, &claims, &state.jwt_encoding_key).map_err(...)
}
```

Constants (`mcp/src/oauth/mod.rs:54-58`):

```rust
pub const JWT_ISSUER: &str = "mcp.mnemonik.xyz";
pub const JWT_AUDIENCE: &str = "mcp";
pub const JWT_TTL_SECS: u64 = 3600;       // 1 hour
```

`/oauth/token` returns `expires_in: JWT_TTL_SECS` (`mcp/src/oauth/mod.rs:1075`).
So the access token lives 1 hour from mint.

### Refresh token?

**No.** grep of `mcp/src/`:
- `oauth/mod.rs:435` — `refresh the token` only appears as part of an
  error message comment, not implementation.
- `mcp.rs:435` — `refresh the token` only in a doc comment about the
  outbound soft-fall path.
- `mcp.rs:1256` — `refreshed in background` is the pricing engine, not
  tokens.

No `refresh_token` flow exists. Token rotation in-session is impossible
today; on expiry the client must rerun the full OAuth ceremony. This is
exactly the gap the user-spec's `mnemonic_request_reauth` fills.

The `~/.mnemonic/token.json` cache (Task 6 of agent-native-distribution,
referenced at `mcp/src/oauth/mod.rs:1065-1071`) is a CLI-side artifact
used by the soft-fall outbound path; HTTP MCP hosts (Claude Desktop) do
NOT read from it.

---

## G. Open question helpers

### Q1 — `recall` with non-null `owner_pubkey` and no JWT

**Answer from code: the `owner_pubkey` field on `arguments` is silently
ignored.** Recall's tenancy is driven entirely by `jwt_sub`. See §C above
(`mcp/src/mcp.rs:1640-1665`): the dispatcher only reads `args["query"]`
and `args["limit"]`. So:

- Anonymous + `arguments.owner_pubkey = "X"` today: returns the
  cross-owner public pool. The `"X"` is dropped on the floor.
- Anonymous + no `owner_pubkey`: same — cross-owner public pool.

Result: "anonymous recall targeted at a specific owner" is not a
supported shape today; the field is undefined-by-omission. The spec can
choose to keep it that way OR formalise it (allow `arguments.owner_pubkey`
to scope recall, with visibility=public filter applied). The current
behaviour leans toward "anonymous recall is global public pool, full
stop."

### Q2 — `request_reauth` URL: same as initial, or marked re-auth=1?

The OAuth surface today has **no notion** of "this is a renewal". See §D:
- `/oauth/authorize` accepts standard PKCE params; no re-auth marker.
- `/oauth/google/start` builds the URL server-side with a fixed param
  set (`mcp/src/oauth/google.rs:487-499`); no re-auth marker.

Adding `?re-auth=1` would be a NEW concept. Mechanically it's free
(query param survives through the redirect), but it would only be useful
if the consent page is updated to read it and render different copy.
That's a frontend change outside this feature's scope.

Pragmatic answer: return the SAME URL as initial. Differentiation can be
done in a follow-up if there's UX demand.

### Q3 — TTL on `request_reauth` URL

Two reusable TTLs in the codebase:

1. `confirmation_token::DEFAULT_TTL = 300s` (5 min) at
   `mcp/src/confirmation_token.rs:40` — used for HMAC-bound write
   confirmations.
2. `oauth::STATE_TTL_SECS = 60s` at `mcp/src/oauth/mod.rs:61` — used
   for the Solana-wallet `/oauth/authorize` pending state.
3. `oauth::CODE_TTL_SECS = 60s` at `mcp/src/oauth/mod.rs:63` — issued
   code TTL.
4. `google::GOOGLE_STATE_TTL_SECS = 600s` (10 min) at
   `mcp/src/oauth/google.rs:78` — Google flow pending state.

The authorize URL ITSELF doesn't have a TTL in the OAuth flow — the
server-side pending state has the TTL, and the URL points at server
state. So the right value to surface to the agent is the pending-state
TTL (60s for Solana flow, 600s for Google). For the spec's
`expires_in`, the cleanest answer is "whichever flow the deployment
uses → its pending-state TTL". A round number like 600s is a safe
default if the spec wants one constant.

`confirmation_token::DEFAULT_TTL`'s 5min is for a different shape
(HMAC-bound write proof) and probably not the right reuse target — the
HMAC key is in-process memory (`mcp/src/confirmation_token.rs:21-22`
notes a restart invalidates everything). For an OAuth URL, that's not
the same property.

---

## Risks & gotchas

### Body double-buffering

The middleware already buffers the body to extract `method` and (for
`tools/call`) the tool name. Adding shape inspection
(`arguments.mode` / `arguments.visibility`) means parsing the body's
`arguments` object too. Bounded at 1 MiB (`MAX_PEEK_BODY` at
`mcp/src/oauth/mod.rs:1307`), so no new attack surface — but the body
must still be re-injected after inspection (currently done at
`mcp/src/oauth/mod.rs:1512` and `:1522`). The new whitelist predicate
needs to thread through both branches without dropping bytes.

### Body parse ALREADY happens, but only for `method` + tool name

`extract_json_rpc_method` (`mcp/src/oauth/mod.rs:1340-1343`) and
`extract_tools_call_name` (`:1351-1360`) each parse the bytes into a
`serde_json::Value`. They're called sequentially — that's 2x JSON parse
per request today. Adding a third parse for `arguments.mode` is cheap
but consider passing a single parsed Value through instead.

### `args["owner_pubkey"]` is currently silently ignored on recall

A request like `{"method":"tools/call", "params":{"name":"mnemonic_recall",
"arguments":{"owner_pubkey":"X", "query":"q"}}}` reaches the dispatcher
and the `owner_pubkey` field is read by neither the dispatcher
(`mcp/src/mcp.rs:1640-1665`) nor `tools::recall` (`tools.rs:1786-1831`).
This may surprise clients who expect targeting. Worth surfacing in
the user-spec's AC6 ("any other tool 401") — recall today does NOT
401 with `owner_pubkey` set (anonymous), it just ignores the field.

### The `STORAGE_MODE` env-var still drives the fallback

`mcp/src/tools.rs:101-117` — when `arguments.mode` is absent,
`resolve_write_mode` falls back on `env_storage_mode`. Spec authors
should be explicit: the whitelist must check for the LITERAL string
`"local"` in `arguments.mode`, NOT trust the fallback. The fallback
exists for backward-compat with the chrome-extension (preserves
byte-for-byte behaviour); whitelisting based on fallback would let an
anonymous caller write `mode`-absent on a `STORAGE_MODE=local` deploy
without intent.

### `visibility` ABSENT on local mode means "private"

`mcp/src/tools.rs:160-178` — `resolve_visibility` returns
`Visibility::default()` (Private) when `args["visibility"]` is absent,
AND **rejects** any present `visibility` value when mode is Local
(AC14). So the safe whitelist shape is:

- `arguments.mode == "local"` AND `arguments.visibility` ABSENT.

NOT `arguments.visibility == "private"` — that triggers AC14 and
returns `-32602 InvalidParams`. The user-spec's framing
"local+private" is semantic ("the row will land as private because
that's the default for local"), not literal.

### Tool count regression assertion

`mcp/src/mcp.rs:1965` asserts "expected 7 MCP tools in tools/list
response". Adding `mnemonic_request_reauth` bumps this to 8. Easy fix,
just remember to update.

### Soft-fall path already handles outbound expiry (not the inbound problem)

The `-32099 TokenExpired` typed error in
`tools::proxy_participate` (`mcp/src/tools.rs:633-635`) handles the
OUTBOUND case where the local Rust binary reads
`~/.mnemonic/token.json` for a participate proxy. This is unrelated to
the HTTP MCP host's inbound JWT expiring. The user-spec's
"`-32099 TokenExpired`" reference in AC3 is correct in spirit (it's the
right error code from the catalogue) but today's actual wire response
for an expired inbound JWT is `-32001 unauthorized: invalid JWT:
ExpiredSignature` — see `mcp/src/oauth/mod.rs:1502-1510` returning
through `jsonrpc_unauthorized` at `:1547`.

If the spec wants the agent to see `-32099 TokenExpired` for expired
inbound JWTs, the middleware needs to distinguish "missing JWT" from
"present-but-expired" and emit `-32099` with `expires_at` populated. That
is a NEW translation point and worth flagging in the tech-spec.

### WWW-Authenticate header still emitted

`mcp/src/oauth/mod.rs:1550-1568` — every 401 from the middleware also
emits `WWW-Authenticate: Bearer realm="https://mcp.mnemonik.xyz",
error="invalid_token", error_description="...",
resource_metadata=".../oauth-protected-resource"`. MCP clients use this
to drive re-auth. If `mnemonic_request_reauth` lands, the WWW-Auth
header on the OTHER 401 paths (whoami, verify, etc.) still steers
"correct" clients to re-OAuth; the new tool is the in-tool path for
clients that can't reattach OAuth mid-session (the Claude Desktop case
the user-spec calls out).

### Mutex around the store — no surprises

The `recall` dispatcher (`mcp/src/mcp.rs:1666-1676`) takes
`state.store.lock().unwrap()` and the lock dies at the end of the block.
No `.await` inside the lock. The whitelisted local-sign path will need
to do the same — see `sign_memory_inline` at `mcp/src/tools.rs:1067-1072`
for the local-write SQLite touch point; it already follows the
"lock, write, drop" discipline.

### `mode` resolution runs TWICE in the middleware-whitelist world

The middleware would parse `arguments.mode` to decide the whitelist;
the dispatcher already re-parses it at `mcp/src/mcp.rs:1173-1195`
(`resolve_write_mode`). Two parses of the same field, two chances to
disagree on edge cases (e.g. whitespace handling). Spec authors should
either:
- Reuse `resolve_write_mode` in the middleware (and convert the
  `JsonRpcError` to a 401 — awkward), OR
- Define the middleware predicate as "literal `Value::String("local")`
  in `arguments.mode`" and rely on the dispatcher's strict parser to
  reject any malformed body that slipped past with the same input.

The second option is safer — the middleware whitelist becomes purely a
"shape matches" gate; the dispatcher's strict parse is the authoritative
validator.

### Public stats route is a different shape from the new whitelist

The spec cites `mcp/src/api.rs:1239` (public counters) as precedent.
That route lives OUTSIDE the bearer-auth subrouter
(`mcp/src/main.rs:1108` registers `/stats` on the main router, with no
middleware layer). The shape the new tool actually fits is
`ALLOWLIST_TOOLS_CALL_NAMES` at `mcp/src/oauth/mod.rs:1372` — the
existing per-tool carve-out that today contains only `mnemonic_recall`.
The user-spec should be aware: it's adding entries to
`ALLOWLIST_TOOLS_CALL_NAMES` AND extending the predicate at
`mcp/src/oauth/mod.rs:1458-1480` to also inspect `arguments` for
`sign_memory`. That's a strictly bigger change than the existing
"`recall` is anonymous" carve-out.

### Test JWT mint helper does not support `exp` < now

`mcp/src/test_support.rs:441-457` hardcodes `exp = now + 3600`. To
exercise an artificially-expired JWT path (AC1 — "token искусственно
протух"), the test harness will need a new helper. Trivially copy
`mint_jwt` and accept `exp_offset: i64` so the test can pass `-1`. The
JWT verify path WILL reject it
(`mcp/src/oauth/mod.rs:410: validation.validate_exp = true`).

---

## §H — Refresh-token rotation touchpoints

Added 2026-06-06. Scope: add OAuth 2.1 refresh-token rotation (Stripe
MCP precedent — 1h access + 1y rolling refresh). Does not disturb
sections A-G; all file:line citations independently verified against
the current tree.

### §H.1 — Current `/oauth/token` shape

**Route registration** (`mcp/src/main.rs:941`):

```rust
.route("/oauth/token", post(oauth::token_handler))
```

Sits behind the per-IP governor (`/oauth/*` bucket,
`mcp/src/main.rs:810-829`) but is URI-allowlisted by the bearer-auth
middleware so it does not require a JWT to call.

**Handler signature** (`mcp/src/oauth/mod.rs:982-987`):

```rust
pub async fn token_handler(
    State(state): State<Arc<OAuthState>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
```

**Request body** (`mcp/src/oauth/mod.rs:946-957`):

```rust
pub struct TokenRequest {
    pub code: String,
    pub code_verifier: String,
    #[serde(default)]
    pub redirect_uri: Option<String>,
}
```

There is NO `grant_type` field today — the handler implicitly treats
every call as `authorization_code`. Refresh-token rotation will need to
make `grant_type` explicit (default to `"authorization_code"` for
back-compat, accept `"refresh_token"` as the new branch).

**Response body** (`mcp/src/oauth/mod.rs:959-967`):

```rust
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub scope: String,
}
```

No `refresh_token` field today. Adding one with
`#[serde(skip_serializing_if = "Option::is_none")]` keeps the wire
format byte-identical for legacy clients (same pattern as
`Claims::google_sub` at `mcp/src/oauth/mod.rs:95-96`).

**Success path** (`mcp/src/oauth/mod.rs:1019-1078`) — quoted key steps:

```rust
let issued = {
    let mut guard = state.codes.lock().expect("codes mutex poisoned");
    guard.pop(&req.code)
};
// ...PKCE + redirect_uri checks...
let token = match issue_jwt_with_google_sub(&state, &issued.sub, issued.google_sub.clone()) {
    Ok(t) => t,
    Err(e) => { return oauth_error(StatusCode::INTERNAL_SERVER_ERROR, ...); }
};
cache_minted_token(&token, &issued.sub);
let body = TokenResponse {
    access_token: token,
    token_type: "Bearer".to_string(),
    expires_in: JWT_TTL_SECS,
    scope: "mcp".to_string(),
};
(StatusCode::OK, Json(body)).into_response()
```

State touched in the success path: `state.codes` (consumed via
`pop`), the JWT signing key (read-only), the on-disk token cache
(`~/.mnemonic/token.json` — best-effort, `cache_minted_token` swallows
errors). No SQLite write today. Adding refresh tokens means the
handler MUST gain access to a connection — see §H.2 for where it
lives.

### §H.2 — Access-token minting

`issue_jwt_with_google_sub` (`mcp/src/oauth/mod.rs:380-397`) is the
ONLY production access-token mint:

```rust
pub fn issue_jwt_with_google_sub(
    state: &OAuthState,
    sub: &str,
    google_sub: Option<String>,
) -> Result<String, String> {
    let now = now_secs();
    let claims = Claims {
        iss: JWT_ISSUER.to_string(),
        aud: JWT_AUDIENCE.to_string(),
        sub: sub.to_string(),
        iat: now,
        exp: now + JWT_TTL_SECS,
        jti: uuid::Uuid::new_v4().to_string(),
        google_sub,
    };
    let header = Header::new(Algorithm::HS256);
    encode(&header, &claims, &state.jwt_encoding_key).map_err(|e| format!("JWT encode failed: {e}"))
}
```

The thin `issue_jwt` (`mcp/src/oauth/mod.rs:373-375`) is a
test-compat shim that delegates here. Other mint helpers:

- `mcp/src/escrow.rs` mints `aud="extension"` JWTs against the SAME
  HS256 secret (accessed via `OAuthState::jwt_encoding_key()` at
  `mcp/src/oauth/mod.rs:251-253`). Not a concern for refresh: a
  refresh exchange should re-mint with `aud="mcp"`, never with
  `aud="extension"`.
- `mcp/tests/_helpers/mod.rs:146-148` (`TestServer::mint_jwt`) — test
  helper only.

The refresh-grant branch should call `issue_jwt_with_google_sub`
(preserving any `google_sub` claim from the original code exchange).

### §H.3 — Storage candidates for refresh-token table

**Architectural rule** (CLAUDE.md): OAuth state lives in `mcp/`, never
in `core/`. The new `refresh_tokens` table MUST live in `mcp/`.

**Existing precedents** (in priority order):

1. **`mcp/src/oauth/google.rs:340-352`** — `migrate_google_identity_links`:

```rust
pub fn migrate_google_identity_links(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS google_identity_links (
            google_sub    TEXT PRIMARY KEY,
            pubkey_base58 TEXT NOT NULL,
            linked_at     INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_google_identity_links_pubkey
            ON google_identity_links(pubkey_base58);",
    )
    .context("create google_identity_links table")?;
    Ok(())
}
```

Called from `mcp/src/main.rs:499`.

2. **`mcp/src/escrow.rs:113-133`** — `migrate_key_escrow_blobs` with
   FK and `MIGRATION_SQL` as a public `const` so tests can read it.
   Called from `mcp/src/main.rs:505`.

Both reuse the same SQLite file as `core::SqliteStore` via
`store.conn()` (`core/src/storage/sqlite.rs:514-517`). The `mcp/`
table sits beside the `core/` tables; only the migration lives in
`mcp/`.

**Recommended placement**: a new `mcp/src/oauth/refresh.rs` module
exporting `migrate_refresh_tokens(&Connection) -> Result<()>` plus
the rotation helpers. Wire it at `main.rs:499` alongside
`migrate_google_identity_links` and `migrate_key_escrow_blobs`.

**IssuedCode in-memory precedent** (`mcp/src/oauth/mod.rs:127-147,
156-160`):

```rust
struct IssuedCode {
    sub: String,
    code_challenge: String,
    redirect_uri: String,
    google_sub: Option<String>,
    exp: u64,
}

pub struct OAuthState {
    pending: Mutex<LruCache<String, PendingAuthorize>>,
    codes: Mutex<LruCache<String, IssuedCode>>,
    clients: Mutex<LruCache<String, RegisteredClient>>,
    jwt_encoding_key: EncodingKey,
    jwt_decoding_key: DecodingKey,
}
```

`IssuedCode` is in-memory LRU with `CODE_TTL_SECS = 60`
(`mcp/src/oauth/mod.rs:63`). Refresh tokens have a much longer TTL
(1y) and MUST survive restart, so an in-memory LRU is the WRONG
shape — SQLite is the correct backing store. The
`IssuedCode`-style API (mint + atomic pop) is still a useful
precedent for the call shape, just backed by a row instead of an LRU
entry.

### §H.4 — Token hashing precedent

`mcp/src/payment.rs:737-744`:

```rust
pub fn hash_api_key(api_key: &str) -> String {
    blake3::hash(api_key.as_bytes()).to_hex().to_string()
}
```

Used both as a quota-counter subject and as the credential-at-rest
identifier in `payment_events` (see comments at `payment.rs:737-742`).
Unit test at `payment.rs:1279-1287` proves the hex form is
deterministic and does not leak the raw key prefix.

The confirmation-token ledger uses HMAC-SHA256 instead
(`mcp/src/confirmation_token.rs:269-285`) because it binds multiple
fields together; that pattern is overkill for refresh tokens (we only
need to recognize whether a presented opaque secret matches a stored
row).

**Recommended**: `blake3::hash(refresh_token_bytes).to_hex()` —
matches `hash_api_key`, sub-microsecond per call, 64-hex-char fixed
width fits a `TEXT PRIMARY KEY` column. Use 32 random bytes
(`rand::thread_rng().fill_bytes` — same call as
`mcp/src/confirmation_token.rs:97-98`) for the raw refresh token,
URL-safe-base64-encode for the wire, hash the raw bytes for the row.

### §H.5 — Test patterns for `/oauth/token`

**Unit tests** (in-module `#[cfg(test)] mod tests` in
`mcp/src/oauth/mod.rs`). The harness lives at
`mcp/src/oauth/mod.rs:1588-1677`:

- `TEST_SECRET` — 32-byte literal (`mcp/src/oauth/mod.rs:1590`).
- `fresh_state()` — `Arc<OAuthState>` factory (`:1592-1594`).
- `make_raw_signed_challenge(kp, state, redirect_uri, code_challenge,
  nonce, exp) -> (hash, sig_b64, cbor_bytes)` (`:1632-1655`).
- `build_authorize_router(state) -> Router` — mounts both
  `/oauth/authorize` AND `/oauth/token` on a single router
  (`:1657-1662`).
- `post_json(app, uri, body) -> (StatusCode, Value)` (`:1664-1677`).

**Positive test** (`mcp/src/oauth/mod.rs:2019-2030` — happy
authorize → token round-trip):

```rust
let app2 = build_authorize_router(st.clone());
let (s2, body2) = post_json(
    app2,
    "/oauth/token",
    serde_json::json!({"code": code, "code_verifier": verifier}),
)
.await;
assert_eq!(s2, StatusCode::OK);
let token = body2["access_token"].as_str().unwrap().to_string();
let claims = verify_jwt(&st, &token).unwrap();
assert_eq!(claims.sub, pubkey);
```

**Negative tests** (already covering several refresh-relevant
adversaries):

- `test_token_invalid_verifier_401` (`mcp/src/oauth/mod.rs:2032-2077`)
  — wrong PKCE verifier → 401.
- `test_token_expired_code_60s_401` (`:2079-2104`) — direct
  insertion of a past-exp `IssuedCode`, then `/oauth/token` → 401.
  This is the template for "expired refresh row → 401" tests.

**Replay test** (precedent for refresh-token single-use semantics) —
not present for `/oauth/token` itself today, but the code is
single-use because `state.codes.lock().pop(&req.code)` removes the
entry atomically (`mcp/src/oauth/mod.rs:1019-1022`). A "code already
used → 401" test exists implicitly via `oauth_loopback.rs` round-trip
flows but no explicit double-spend assertion. Worth adding for the
refresh path.

**Integration tests** (`mcp/tests/`):

- `mcp/tests/oauth_flow.rs:53-183` — end-to-end `/oauth/authorize`
  → `/oauth/token` round-trip using a custom Router that mounts both
  handlers.
- `mcp/tests/oauth_google.rs:207` — same pattern for the Google
  OAuth path.
- `mcp/tests/oauth_loopback.rs:60` — round-trip through the
  Rust-side token cache.

**No `TestServer::redeem_code` helper exists.** The shared
`TestServer` at `mcp/tests/_helpers/mod.rs:104-126` only mounts
`/mcp`, `/api/pending/{correlation_id}`, and `/api/sign-callback` —
NOT `/oauth/token`. Existing OAuth integration tests build their own
small router each time. Refresh-grant integration tests should either
(a) extend `TestServerBuilder` to also mount `/oauth/token` (cleanest
— one place to read), or (b) keep the per-test mini-router pattern.
Recommend (a) so the refresh-grant test can compose with the existing
`call_tool` path that exercises `/mcp` with the freshly-rotated
access token.

### §H.6 — Claims schema impact

`Claims` struct (`mcp/src/oauth/mod.rs:76-97`):

```rust
pub struct Claims {
    pub iss: String,
    pub aud: String,
    pub sub: String,
    pub iat: u64,
    pub exp: u64,
    pub jti: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google_sub: Option<String>,
}
```

NO `token_type` or `scope` field. The `aud` claim is the only
discriminator — `mcp/src/oauth/mod.rs:419-421` rejects anything
other than `"mcp"`, and `mcp/src/escrow.rs` reuses the same struct
with `aud="extension"`.

**Implication for "refresh-as-JWT"**: if the refresh token is itself
a JWT, mint it with a distinct `aud` (e.g. `"mcp+refresh"`) so it
CANNOT be presented to `/mcp` as an access token — the bearer-auth
middleware's `verify_jwt` (`:401-423`) calls
`validation.set_audience(&[JWT_AUDIENCE])` with `JWT_AUDIENCE = "mcp"`
and rejects mismatches. Easier alternative (Stripe MCP's choice): keep
refresh as an opaque random secret hashed in SQLite, no `Claims`
change at all.

`Claims` is `pub` and is read by `mcp/src/oauth/google.rs`,
`mcp/src/escrow.rs`, `mcp/src/tools.rs`, and ~12 integration tests
— adding a field is feasible (the existing `google_sub` migration
shows the pattern: `#[serde(default, skip_serializing_if =
"Option::is_none")]` keeps the wire byte-identical) but every
inspection site needs an audit.

### §H.7 — Background eviction precedent

`mcp/src/confirmation_token.rs:259-267`:

```rust
pub fn start_evictor(ledger: Arc<ConfirmationLedger>) -> tokio::task::JoinHandle<()> {
    let tick = ledger.evict_tick();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tick).await;
            ledger.evict_expired();
        }
    })
}
```

`DEFAULT_EVICT_TICK = 60s`, `DEFAULT_TTL = 300s`
(`mcp/src/confirmation_token.rs:40-43`). Spawned at
`mcp/src/main.rs:635`:

```rust
let _confirmation_evictor = confirmation_token::start_evictor(confirmation_ledger);
```

For refresh tokens with 1y TTL, a per-minute sweep is overkill.
Recommend a much slower tick (e.g. 1h) plus an opportunistic
"delete expired siblings of this `sub`" inside the rotation path so
abandoned rows clear without waiting for the next tick.

### §H.8 — Discovery metadata (RFC 8414)

`mcp/src/oauth/mod.rs:1178-1191`:

```rust
pub async fn oauth_authorization_server_metadata() -> Response {
    let body = serde_json::json!({
        "issuer": SERVER_ORIGIN,
        "authorization_endpoint": format!("{SERVER_ORIGIN}/oauth/authorize"),
        "token_endpoint": format!("{SERVER_ORIGIN}/oauth/token"),
        "registration_endpoint": format!("{SERVER_ORIGIN}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": ["mcp"],
    });
    (StatusCode::OK, Json(body)).into_response()
}
```

NO `revocation_endpoint`, `introspection_endpoint`, or
`refresh_token_*` fields today. The refresh-rotation change adds:

- `"refresh_token"` to `grant_types_supported`.
- (Optional, RFC 7009) `revocation_endpoint: "{SERVER_ORIGIN}/oauth/revoke"`.

Sibling endpoint `oauth_protected_resource_metadata`
(`:1195-1203`) and the path-specific
`oauth_protected_resource_metadata_mcp` (`:1215-1223`) describe the
RESOURCE; they do not advertise grant types and need no change.

Routes wired at `mcp/src/main.rs:972-984` — all three sit on the
`well_known_routes` subrouter with no auth and no governor.

### §H.9 — RFC 7009 revocation endpoint (optional)

`grep -rn "revoke\|revocation" mcp/src` returns ZERO OAuth-related
hits — no `/oauth/revoke` route is registered today. The two matches
in `mcp/src/escrow.rs:14` and `:751` refer to KEY-ESCROW
DELETE/revocation, an unrelated concept.

**Insertion point**: `mcp/src/main.rs:941` (next to
`/oauth/token`). The handler would parse a form-encoded body
(`token`, optional `token_type_hint=refresh_token|access_token`),
hash the token, and delete the matching row from `refresh_tokens` (or
mark it revoked). RFC 7009 §2.2 mandates a 200 response even for
unknown tokens to prevent enumeration — important detail for the
tech-spec.

### §H.10 — Refresh-token cleanup hooks

`logout` command (`mcp/src/main.rs:315-330`) only deletes the
on-disk `~/.mnemonic/token.json` cache via
`mnemonic_core::identity::delete_token()`. It runs in the CLI
process BEFORE `McpState::build` and never touches the server's
SQLite store. There is no "user logged out" server-side signal today
— refresh tokens for that user will simply keep working until they
expire or are explicitly revoked.

That's fine for V1: revocation comes from the per-user revocation
API (RFC 7009 endpoint above) and from rotation itself (the
old-refresh-revoked-on-use semantic). No new hooks needed.

One related cleanup site worth knowing about: the consent flow at
`mcp/src/oauth/mod.rs:921-934` already mints `IssuedCode` rows via
`state.codes.lock().put(...)`. The refresh-rotation path should
follow the same atomic-write discipline (begin-immediate +
single-statement INSERT/DELETE so a crashed rotation cannot leave
both old and new refresh rows valid). See `payment.rs:478-505` for
the precedent of wrapping balance UPDATE + payment_events INSERT in
`BEGIN IMMEDIATE`.

### §H — Risks & gotchas

1. **`token_handler` body type is `Bytes`, not `Form`/`Json`.** The
   handler hand-rolls content-type dispatch at
   `mcp/src/oauth/mod.rs:990-1017`. Adding a `grant_type` branch must
   keep both code paths (form-encoded for VS Code/Claude.ai, JSON for
   Cursor) and must NOT use axum's `#[derive(Deserialize)]` form
   extractors directly. Mirror the existing two-arm `if
   ct.starts_with("application/x-www-form-urlencoded")` pattern.

2. **`TokenRequest` has no `grant_type` field today.** Adding
   `grant_type: Option<String>` with `Option::unwrap_or("authorization_code")`
   preserves back-compat for the legacy webapp test fixtures (and the
   `scripts/test-oauth-flow.sh` shell script) that send only `code` +
   `code_verifier`. A `grant_type=refresh_token` branch requires a
   DIFFERENT body shape (`refresh_token: String`, no `code`/`code_verifier`).
   Cleanest approach: deserialize into an untagged enum or parse
   `grant_type` first, then re-parse the body into the
   variant-specific struct.

3. **PRAGMA `foreign_keys=ON` is per-connection.**
   `core/src/storage/sqlite.rs:484-487` sets it in
   `SqliteStore::open` and `:504` in `in_memory`. Any new refresh
   table that uses `FOREIGN KEY` (e.g. linking back to
   `google_identity_links` or a hypothetical `users` table) inherits
   this for free — but tests that build a raw `Connection` (e.g.
   `core/src/storage/sqlite.rs:1348-1351` `open_legacy_attestations_db`)
   will silently skip FK enforcement.

4. **Migration ordering matters.** At `mcp/src/main.rs:499` the
   `google_identity_links` migration runs FIRST (key_escrow_blobs has
   an FK to it). If the refresh table FKs to anything in `mcp/`'s
   schema, place the migration AFTER its dependency. The `core/`
   schema (`SCHEMA` const at `core/src/storage/sqlite.rs:13-83`) is
   already created by `SqliteStore::open` at `:488` BEFORE
   `main.rs:499` runs, so FK-ing to a `core/` table is also safe.

5. **`!Send` connection across `.await`.** Project CLAUDE.md hard
   rule — `rusqlite::Connection` is `!Send`; the codebase wraps it in
   `std::sync::Mutex`. The new rotation function MUST take the
   `Connection` by ref (or via the existing `McpState::store` mutex)
   and MUST NOT hold the lock across any `.await`. See
   `mcp/src/payment.rs:399-505` (`deduct_balance`) for the canonical
   pattern: take the mutex, run the whole BEGIN/COMMIT
   synchronously, drop the guard, then return.

6. **Token cache file (`~/.mnemonic/token.json`) only stores the
   access token.** `cache_minted_token`
   (`mcp/src/oauth/mod.rs:1071, 1101`) writes the JWT and a derived
   `expires_at`. It does NOT carry a refresh token. The
   refresh-rotation tech-spec must extend the on-disk schema or add a
   sibling file — and the security contract at `:1086-1097` (the
   warning that `sub` must be PKCE-validated before caching) applies
   equally to any cached refresh token. The agent-native distribution
   feature explicitly chose "no OS keychain in V1"
   (Decision 7) — refresh tokens at rest will inherit the same
   threat model (mode-0600 file in `$HOME/.mnemonic/`).

7. **HS256 secret is the SAME for `aud=mcp` and `aud=extension`.**
   `OAuthState::jwt_encoding_key()` (`mcp/src/oauth/mod.rs:251-253`)
   is exposed precisely so `escrow.rs` can mint extension JWTs with
   the same key. If refresh tokens are minted as JWTs (alternative to
   opaque tokens), they share this key too. An attacker who steals
   the key can forge any token type. Opaque refresh tokens dodge this
   coupling — a stolen `MCP_JWT_SECRET` does NOT let the attacker
   forge a refresh-token row in SQLite.

8. **No explicit "code already used" replay test.** The single-use
   semantic relies on `LruCache::pop` at
   `mcp/src/oauth/mod.rs:1019-1022`. For refresh tokens, single-use
   is THE security property (RFC 6749 §10.4 — refresh-token rotation
   detects stolen refresh tokens via reuse → revoke all descendants).
   The tech-spec MUST require an explicit "old refresh presented after
   rotation → 401 AND revoke the entire token family" test, and the
   implementation MUST persist `rotated_to` so the family can be
   traversed when reuse is detected.

9. **`grant_types_supported` is read at request time, not cached.**
   `oauth_authorization_server_metadata`
   (`mcp/src/oauth/mod.rs:1178-1191`) rebuilds the JSON literal on
   every call. Toggling refresh support behind an env var (e.g.
   `OAUTH_REFRESH_DISABLED=1`) is straightforward — just branch on
   the env var when constructing the array. But: MCP clients
   typically cache `/.well-known/oauth-authorization-server`
   responses, so flipping the env var on a live server won't
   immediately propagate to existing clients.

10. **Rate limit on `/oauth/*` is per-IP.**
    `mcp/src/main.rs:810-829` — 5 burst + 1 req/s refill. A refresh
    grant is one POST per access-token expiry (≈ once per hour per
    client). Per-IP is fine for that volume, but a CI run that
    refreshes 6 clients concurrently from the same IP may trip the
    bucket. If that becomes a problem in tests, the existing
    `OAUTH_RATELIMIT_DISABLE=1` env-var escape hatch
    (`mcp/src/main.rs:826-830`) is the documented bypass.

---

## §I — Implementation-level details

## Updated: 2026-06-06

Added at tech-spec stage. Scope: refresh-token rotation (D1-D17 + D13.1
locked). Quotes every call site the implementer touches and recommends
one specific approach per section. Does not re-cover §A-§H.

### §I.1 — `JWT_TTL_SECS` env-plumbing surface

**The const** (`mcp/src/oauth/mod.rs:58`):

```rust
pub const JWT_TTL_SECS: u64 = 3600;
```

**Every read site** across the workspace (`grep -rn JWT_TTL_SECS`):

1. `mcp/src/oauth/mod.rs:391` — `exp: now + JWT_TTL_SECS` in
   `issue_jwt_with_google_sub`. The canonical mint for `aud="mcp"`.
2. `mcp/src/oauth/mod.rs:1075` — `expires_in: JWT_TTL_SECS` in
   `token_handler`'s `TokenResponse` (advertised TTL on the auth-code
   exchange). The refresh-grant branch needs the same value.
3. `mcp/src/oauth/mod.rs:1113` — `cache_minted_token`'s fallback when
   the JWT's `exp` claim is unparseable: `now + JWT_TTL_SECS`.
4. `mcp/src/oauth/mod.rs:1124` — same fallback when `exp` claim is
   missing entirely.
5. `mcp/src/escrow.rs:59` — `use crate::oauth::{..., JWT_TTL_SECS};`.
6. `mcp/src/escrow.rs:511` — `expires_in: JWT_TTL_SECS` for the
   extension-bootstrap response.
7. `mcp/src/escrow.rs:797` — `exp: now + JWT_TTL_SECS` in extension
   JWT mint (`aud="extension"`).

Test-only reads (not impl-critical):

- `mcp/tests/oauth_loopback.rs:35,353-360` — uses the constant to
  assert the cached token's `expires_at` is within `JWT_TTL_SECS` of
  now. If we change the const at runtime, this test's window assertion
  still holds because it reads the same import.
- `core/src/identity/token_store.rs:11` — doc comment reference only.

**Recommendation: Option (b) — process-global `OnceCell<u64>`** seeded
at startup from env, defaulting to 3600. Rationale:

- Read sites are scattered across `oauth/mod.rs`, `escrow.rs` (with the
  field literal `JWT_TTL_SECS` baked into doc-comments). Replacing the
  `pub const` with a `state.jwt_ttl_secs` field (Option a) ripples
  through `escrow.rs:797` (no `OAuthState` in scope at that mint site —
  `mint_extension_jwt` takes `&OAuthState` already, so this is
  tractable but ugly), `cache_minted_token` (takes only `jwt` + `sub`),
  and every test fixture that reads `JWT_TTL_SECS` as a constant. ~10
  call-site rewrites.
- Option (b) keeps the import shape (`use crate::oauth::JWT_TTL_SECS`)
  — convert from `pub const` to `pub fn jwt_ttl_secs() -> u64` that
  reads a `static JWT_TTL_OVERRIDE: OnceCell<u64>` and falls back to
  `3600`. Initialise in `main.rs::run_http` (or `run_stdio`) from the
  optional env var `MCP_JWT_TTL_SECS`. Zero behavioural change when
  the env var is absent. Token cache and tests keep reading the
  function and the value is consistent process-wide.
- Option (c) — hard-coded dev patch — works for the immediate dev-deploy
  R1 scenario but loses any "configurable for staging" property and
  leaves a known-bad value in the binary if anyone reuses it.

**Minimal change** (Option b skeleton):

```rust
// mcp/src/oauth/mod.rs:58
use once_cell::sync::OnceCell;

static JWT_TTL_OVERRIDE: OnceCell<u64> = OnceCell::new();

/// Initialise the JWT-TTL override once at startup. Idempotent — second
/// call is a no-op. Read by `jwt_ttl_secs()`.
pub fn init_jwt_ttl_secs(seconds: u64) {
    let _ = JWT_TTL_OVERRIDE.set(seconds);
}

pub fn jwt_ttl_secs() -> u64 {
    *JWT_TTL_OVERRIDE.get().unwrap_or(&3600)
}

#[deprecated = "use jwt_ttl_secs() — preserves const for back-compat reads"]
pub const JWT_TTL_SECS: u64 = 3600;
```

Then rewrite the 7 production read sites above to call `jwt_ttl_secs()`
instead of `JWT_TTL_SECS`. `once_cell` is already in the workspace tree
(`core/Cargo.toml`). The deprecation hint keeps the test file from
silently picking up the wrong value until the test is updated; CI
clippy gate (`-D warnings`) will fail unless the test is also migrated
— which is the desired behaviour for a constant that no longer
reflects runtime truth.

### §I.2 — `TokenRequest` struct widening

**Current shape** (`mcp/src/oauth/mod.rs:946-957`):

```rust
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub code: String,
    pub code_verifier: String,
    /// `redirect_uri` from the original authorize request. Optional for
    /// backward compatibility with clients that omit it (legacy webapp,
    /// integration tests). When present, MUST equal the value bound at
    /// `/oauth/authorize` time — RFC 6749 §4.1.3 + RFC 7636 §4.4 require this
    /// equality check to defeat a swap-redirect attack on a leaked code.
    #[serde(default)]
    pub redirect_uri: Option<String>,
}
```

`code` + `code_verifier` are non-optional today — `serde_json::from_slice`
or `serde_urlencoded::from_bytes` will fail with HTTP 400 if either is
missing. Adding refresh requires those to become optional too (a refresh
grant has neither).

**Content-type dispatch** (`mcp/src/oauth/mod.rs:990-1017`) — verbatim:

```rust
let ct = headers
    .get(axum::http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .unwrap_or("application/json")
    .to_lowercase();

let req: TokenRequest = if ct.starts_with("application/x-www-form-urlencoded") {
    match serde_urlencoded::from_bytes(&body) {
        Ok(r) => r,
        Err(e) => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                &format!("token request form parse failed: {e}"),
            );
        }
    }
} else {
    // JSON (or unknown content-type — JSON is our default).
    match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                &format!("token request JSON parse failed: {e}"),
            );
        }
    }
};
```

**No grant_type branching today.** The handler implicitly assumes
`authorization_code` and immediately calls `state.codes.lock().pop(&req.code)`
at `mod.rs:1019-1022`.

**Recommended new shape**:

```rust
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    /// OAuth 2.1 grant_type. Defaults to `"authorization_code"` for
    /// legacy clients that omit it (existing `scripts/test-oauth-flow.sh`,
    /// integration tests, the webapp's first redeem call).
    #[serde(default = "default_grant_type")]
    pub grant_type: String,
    // ── authorization_code branch ────────────────────────────────────
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub code_verifier: Option<String>,
    #[serde(default)]
    pub redirect_uri: Option<String>,
    // ── refresh_token branch ─────────────────────────────────────────
    #[serde(default)]
    pub refresh_token: Option<String>,
}

fn default_grant_type() -> String { "authorization_code".to_string() }
```

Then in `token_handler` after parsing, dispatch on `req.grant_type`:

```rust
match req.grant_type.as_str() {
    "authorization_code" => {
        let code = req.code.as_deref().unwrap_or("");
        let verifier = req.code_verifier.as_deref().unwrap_or("");
        if code.is_empty() || verifier.is_empty() {
            return oauth_error(StatusCode::BAD_REQUEST, "invalid_request: code and code_verifier required");
        }
        // existing path...
    }
    "refresh_token" => {
        let rt = req.refresh_token.as_deref().unwrap_or("");
        if rt.is_empty() {
            // AC13: missing or empty refresh_token → 400 invalid_request.
            return oauth_error(StatusCode::BAD_REQUEST, "invalid_request: refresh_token required");
        }
        return handle_refresh_grant(&state, rt).await;
    }
    other => {
        return oauth_error(StatusCode::BAD_REQUEST,
            &format!("unsupported_grant_type: {other}"));
    }
}
```

**Reusable 400 builder** — `oauth_error` at `mod.rs:1158`:

```rust
fn oauth_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({"error": msg}))).into_response()
}
```

Existing 400 sites that demonstrate the convention: `mod.rs:496, 502,
508, 516, 523, 552, 573, 861, 870, 878, 883, 897, 1000, 1011, 1041`.
Mirror this — `oauth_error(StatusCode::BAD_REQUEST, "invalid_request: ...")`.

**Format note** (RFC 6749 §5.2): a strict OAuth client expects the
error JSON to use `{"error": "invalid_request", "error_description": "..."}`.
Today's `oauth_error` flattens both into a single `error` string. The
existing behaviour is not RFC-compliant but is consistent with the rest
of the handler — keeping it consistent is the safer choice for V1; a
follow-up can split the shape.

### §I.3 — `mcp/src/oauth/refresh.rs` module skeleton

**Module-level shape** (recommended public surface):

```rust
//! Refresh-token rotation. OAuth 2.1 + reuse-detection (D1-D17).
//!
//! - Opaque tokens (32 random bytes, URL-safe-base64 on the wire).
//! - Blake3 hash with per-deploy salt at rest (D2; mirrors `payment::hash_api_key`).
//! - 1-year rolling TTL, 5s reuse-interval grace, per-grant family_id (UUID).
//! - BEGIN IMMEDIATE atomic rotation (mirrors `payment::deduct_balance`).

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub const REFRESH_TTL_SECS: u64 = 365 * 24 * 3600;     // 1 year (D3)
pub const REUSE_INTERVAL_SECS: u64 = 5;                 // D5 (Auth0)
pub const EVICTOR_TICK_SECS: u64 = 3600;                // D7 hourly sweep

/// Newly-minted token, one-shot. `plaintext` is the URL-safe-base64
/// raw refresh token; surface it on the HTTP response then drop the
/// struct — there is no way to reconstruct it once `Drop` runs.
pub struct RefreshToken {
    pub plaintext: String,
    pub token_hash: String,        // hex(blake3(salt || raw_bytes))
    pub family_id: String,         // UUID
    pub expires_at: u64,           // unix seconds
}

/// One row in `refresh_tokens`. Used by family-revoke + reuse-detection.
#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    pub token_hash: String,
    pub sub: String,
    pub google_sub: Option<String>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub revoked: bool,
    pub rotated_to: Option<String>,
    pub family_id: String,
    pub rotated_at: Option<u64>,    // populated when revoked=1
}

/// Mint a fresh refresh token for the just-redeemed authorization_code.
/// Generates a new `family_id` (every authorize handshake is its own
/// device family — D6). Returns the plaintext via `RefreshToken`.
pub fn mint_for_authorization_code(
    conn: &Connection,
    salt: &[u8; 32],
    sub: &str,
    google_sub: Option<&str>,
) -> Result<RefreshToken>;

/// Atomic rotation under BEGIN IMMEDIATE. Behaviours:
/// - Unknown plaintext       → Err("invalid_grant").
/// - Expired row             → Err("invalid_grant"), NO family revoke.
/// - Revoked outside 5s grace → revoke entire family, Err("invalid_grant").
/// - Revoked within 5s grace  → return the existing successor (idempotent retry).
/// - Valid row                → mark old revoked + insert new, return new token.
pub fn rotate(
    conn: &Connection,
    salt: &[u8; 32],
    plaintext: &str,
) -> Result<(RefreshToken, String /* sub */, Option<String> /* google_sub */)>;

/// Mark every row in `family_id` as revoked. Called by `rotate` when a
/// reuse-after-grace is detected (D5).
pub fn family_revoke(conn: &Connection, family_id: &str) -> Result<()>;

/// Idempotent migration. Called once at startup from `main.rs::run_http`
/// (and any test harness that wants the table).
pub fn migrate_refresh_tokens(conn: &Connection) -> Result<()>;

/// Delete every row with `expires_at <= now`. Called from the hourly
/// background evictor. Returns the number of rows removed.
pub fn evict_expired(conn: &Connection) -> Result<usize>;
```

**Migration function — mirrors `escrow::migrate_key_escrow_blobs`
verbatim** (`mcp/src/escrow.rs:113-133`):

```rust
pub fn migrate_refresh_tokens(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(MIGRATION_SQL)
        .context("create refresh_tokens table")?;
    Ok(())
}

pub const MIGRATION_SQL: &str = "CREATE TABLE IF NOT EXISTS refresh_tokens (
    token_hash   TEXT PRIMARY KEY,
    sub          TEXT NOT NULL,
    google_sub   TEXT,
    issued_at    INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    revoked      INTEGER NOT NULL DEFAULT 0,
    rotated_at   INTEGER,
    rotated_to   TEXT REFERENCES refresh_tokens(token_hash) ON DELETE SET NULL,
    family_id    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_refresh_tokens_family
    ON refresh_tokens(family_id);
CREATE INDEX IF NOT EXISTS idx_refresh_tokens_expires_at
    ON refresh_tokens(expires_at);
CREATE INDEX IF NOT EXISTS idx_refresh_tokens_sub
    ON refresh_tokens(sub);";
```

Notes on the DDL:

- 9 columns total (8 from D-decisions + `rotated_at INTEGER` so the
  5s reuse-window comparison runs against a stored timestamp, not a
  computed value). Documented as part of D5 in `decisions.md`; spec
  author should keep that adjustment visible.
- `rotated_to` is a self-FK with `ON DELETE SET NULL` so a family
  cascade-delete (post-eviction) does not orphan a pointer. SQLite
  enforces FKs only with `PRAGMA foreign_keys=ON`; the connection is
  already configured for this in `core/src/storage/sqlite.rs:484-487`.
- `idx_refresh_tokens_family` is the hot path for `family_revoke`
  (`UPDATE ... WHERE family_id = ?`).
- `idx_refresh_tokens_expires_at` is the hot path for the hourly
  evictor (`DELETE WHERE expires_at <= ?`).
- `idx_refresh_tokens_sub` lets the eventual revocation endpoint
  (out-of-scope V1, but cheap to index now) list a user's grants.

**Per-function SQL sketches**:

- `mint_for_authorization_code`:
  ```
  INSERT INTO refresh_tokens
    (token_hash, sub, google_sub, issued_at, expires_at, revoked, family_id)
    VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)
  ```

- `rotate` — see §I.5 for the full BEGIN IMMEDIATE walk-through.

- `family_revoke`:
  ```
  UPDATE refresh_tokens
     SET revoked = 1, rotated_at = ?2
   WHERE family_id = ?1 AND revoked = 0
  ```

- `evict_expired`:
  ```
  DELETE FROM refresh_tokens WHERE expires_at <= ?1
  ```

### §I.4 — `OAuthState` extension

**Current shape** (`mcp/src/oauth/mod.rs:157-164`):

```rust
pub struct OAuthState {
    pending: Mutex<LruCache<String, PendingAuthorize>>,
    codes: Mutex<LruCache<String, IssuedCode>>,
    clients: Mutex<LruCache<String, RegisteredClient>>,
    jwt_encoding_key: EncodingKey,
    jwt_decoding_key: DecodingKey,
}
```

**Recommended additions**:

```rust
pub struct OAuthState {
    pending: Mutex<LruCache<String, PendingAuthorize>>,
    codes: Mutex<LruCache<String, IssuedCode>>,
    clients: Mutex<LruCache<String, RegisteredClient>>,
    jwt_encoding_key: EncodingKey,
    jwt_decoding_key: DecodingKey,
    // ── NEW for refresh-token rotation ──────────────────────────────
    /// Shared with `McpState::store` — same `Connection` behind the same
    /// `Mutex`. Refresh-token rotation runs under BEGIN IMMEDIATE on
    /// this connection so reads and writes serialise against the other
    /// `mcp/`-owned tables (`google_identity_links`, `key_escrow_blobs`,
    /// `payment_events`).
    pub(crate) refresh_store: Arc<Mutex<rusqlite::Connection>>,
    /// 32-byte per-deploy salt. Mixed into blake3 before the row lookup
    /// (`token_hash = hex(blake3(salt || raw_bytes))`) so a snapshot of
    /// `refresh_tokens` from one deploy cannot be replayed against
    /// another deploy that knows the same plaintext (D2).
    pub(crate) refresh_salt: [u8; 32],
}
```

**Salt approach precedent** (`mcp/src/payment.rs:737-744`):

```rust
pub fn hash_api_key(api_key: &str) -> String {
    blake3::hash(api_key.as_bytes()).to_hex().to_string()
}
```

`payment.rs` uses unsalted blake3 because the API-key threat model is
"server compromise → all keys stolen anyway" — the hash there is just a
CWE-312 hygiene measure. Refresh tokens are different: they are
client-presented credentials with a 1-year lifetime. A per-deploy salt
costs nothing and prevents cross-deploy rainbow-table reuse if a snapshot
leaks. Stored on `OAuthState`, seeded from env (`MCP_REFRESH_TOKEN_SALT`,
base64-decoded to ≥32 bytes; if missing, generate fresh on every boot
and warn — same pattern as `confirmation_token::ConfirmationLedger::new`
at `confirmation_token.rs:90-105`).

**`OAuthState::new` call sites** (4 in production + test code):

1. `mcp/src/main.rs:791` — production HTTP path:
   ```rust
   let oauth_state = Arc::new(oauth::OAuthState::new(&secret));
   ```
   Threading: `OAuthState::new` signature widens to
   `new(secret: &[u8], store: Arc<Mutex<Connection>>, refresh_salt: [u8; 32])`.
   In `main.rs` `store` already lives inside `state` at the same scope
   (`state.store` lock is moved by `Arc<McpState>`); the simplest wiring
   keeps a separate `Arc<Mutex<Connection>>` that BOTH `OAuthState` and
   `McpState` hold:
   ```rust
   let store_arc = Arc::new(std::sync::Mutex::new(SqliteStore::open(...)?));
   let oauth_state = Arc::new(oauth::OAuthState::new(
       &secret,
       store_arc.clone(),
       load_refresh_salt()?,
   ));
   // McpState then takes store_arc.clone() instead of the Mutex<SqliteStore> literal.
   ```
   Alternative: lift only the inner `Connection` (since `SqliteStore`
   exposes `conn() -> &Connection` at `core/src/storage/sqlite.rs:514-517`)
   — requires a small wrapper because `Connection` is `!Send` and lifetime
   borrowed-out-of-Mutex is awkward. Prefer the wrapping approach.

2. `mcp/tests/_helpers/mod.rs:111` — test harness:
   ```rust
   let oauth_state = Arc::new(OAuthState::new(TEST_JWT_SECRET));
   ```
   The `mock_state_with` factory (called one line above) builds the
   SQLite store; lift its connection out into the same `Arc<Mutex>` so
   both can share it. A fixed test salt (32 bytes of `0xAB`) keeps test
   determinism.

3. `mcp/src/oauth/mod.rs:1593` — unit-test `fresh_state()`:
   ```rust
   fn fresh_state() -> Arc<OAuthState> {
       Arc::new(OAuthState::new(TEST_SECRET))
   }
   ```
   Same shape — needs an in-memory SQLite connection. Reuse
   `rusqlite::Connection::open_in_memory()?` then run
   `migrate_refresh_tokens` before returning. Existing pattern lives
   at `core/src/storage/sqlite.rs:504` (`in_memory`).

4. `mcp/src/mcp.rs:1838` — internal test fixture; same treatment as (2).

**Why `Arc<Mutex<Connection>>` not `Arc<Mutex<SqliteStore>>`**: keeps the
new module from depending on `mnemonic_core::storage::SqliteStore`
constructors. The `Connection` is the abstraction shared with the
other `mcp/`-owned tables — `escrow.rs` and `oauth/google.rs` already
take `&rusqlite::Connection` directly (`escrow.rs:113`,
`google.rs:340`).

### §I.5 — `BEGIN IMMEDIATE` rotation transaction

**Canonical precedent** (`mcp/src/payment.rs:478-505`,
`deduct_balance` / `credit_deposit`) — quoted in §H.10 already.

**Adapted for `rotate`**:

```rust
pub fn rotate(
    conn: &Connection,
    salt: &[u8; 32],
    plaintext: &str,
) -> Result<(RefreshToken, String, Option<String>)> {
    let now = now_secs();
    let presented_hash = hash_refresh_token(salt, plaintext);

    conn.execute("BEGIN IMMEDIATE", [])?;

    // 1. Look up the presented row. BEGIN IMMEDIATE already holds the
    //    write lock; no concurrent rotation can race past this read.
    let row: Option<RefreshTokenRecord> = conn.query_row(
        "SELECT token_hash, sub, google_sub, issued_at, expires_at,
                revoked, rotated_at, rotated_to, family_id
           FROM refresh_tokens WHERE token_hash = ?1",
        params![presented_hash],
        |r| Ok(RefreshTokenRecord {
            token_hash: r.get(0)?, sub: r.get(1)?, google_sub: r.get(2)?,
            issued_at: r.get(3)?, expires_at: r.get(4)?,
            revoked: r.get::<_, i64>(5)? != 0,
            rotated_at: r.get(6)?, rotated_to: r.get(7)?, family_id: r.get(8)?,
        }),
    ).optional()?;

    let row = match row {
        Some(r) => r,
        None => {
            let _ = conn.execute("ROLLBACK", []);
            anyhow::bail!("invalid_grant: unknown refresh_token");
        }
    };

    // 2. Expired (AC5) — reject, do NOT revoke family.
    if now > row.expires_at {
        let _ = conn.execute("ROLLBACK", []);
        anyhow::bail!("invalid_grant: refresh_token expired");
    }

    // 3. Revoked branch (AC3 / AC4 / AC12).
    if row.revoked {
        let rotated_at = row.rotated_at.unwrap_or(0);
        if now <= rotated_at + REUSE_INTERVAL_SECS {
            // AC12: idempotent retry within 5s grace — return the existing
            // successor pair. Look it up via rotated_to.
            let successor_hash = row.rotated_to.clone()
                .ok_or_else(|| anyhow::anyhow!("revoked row missing rotated_to"))?;
            let successor: RefreshTokenRecord = conn.query_row(
                "SELECT ... FROM refresh_tokens WHERE token_hash = ?1",
                params![successor_hash], /* same row shape */)?;
            conn.execute("COMMIT", [])?;
            // CAVEAT: we cannot return the plaintext here — it was burned
            // at the original rotation. The grace-window idempotent retry
            // semantic is "the request that wins the race gets the new
            // plaintext; subsequent retries within 5s get the SAME 200
            // response (re-issued JWT, same `refresh_token` value re-emitted).
            // Implementation choice: stash the successor plaintext in an
            // in-memory LRU keyed by old_hash, TTL = REUSE_INTERVAL_SECS.
            // Falls back to invalid_grant after the LRU drops.
            // ALTERNATIVE: have rotate return `Err(ReuseRetry)` and let
            // the handler decide — cleaner, recommended.
            anyhow::bail!("retry_within_reuse_interval");
        } else {
            // AC4: replay outside grace → revoke entire family.
            family_revoke(conn, &row.family_id)?;
            conn.execute("COMMIT", [])?;
            anyhow::bail!("invalid_grant: refresh_token reuse detected; family revoked");
        }
    }

    // 4. Happy path — mint new, mark old revoked, INSERT new.
    let new = mint_refresh_token(salt, &row.sub, row.google_sub.as_deref(),
                                  &row.family_id, now)?;
    conn.execute(
        "UPDATE refresh_tokens
            SET revoked = 1, rotated_at = ?2, rotated_to = ?3
          WHERE token_hash = ?1",
        params![row.token_hash, now, new.token_hash],
    )?;
    conn.execute(
        "INSERT INTO refresh_tokens
            (token_hash, sub, google_sub, issued_at, expires_at, revoked, family_id)
            VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
        params![new.token_hash, row.sub, row.google_sub,
                now, now + REFRESH_TTL_SECS, row.family_id],
    )?;

    conn.execute("COMMIT", [])?;
    Ok((new, row.sub, row.google_sub))
}
```

**`!Send` / `.await` discipline**: `Connection` is `!Send`. The handler
holds the lock through the full BEGIN/COMMIT and DOES NOT `.await`
during the transaction — exactly matching `payment::deduct_balance` at
`payment.rs:419-473`. The handler signature stays sync inside the
locked region:

```rust
async fn handle_refresh_grant(state: &Arc<OAuthState>, plaintext: &str) -> Response {
    let result = {
        // SCOPED guard — drops before the `.await` on response build.
        let conn = state.refresh_store.lock().expect("refresh store mutex poisoned");
        refresh::rotate(&conn, &state.refresh_salt, plaintext)
    };
    match result {
        Ok((new_refresh, sub, google_sub)) => {
            // issue_jwt_with_google_sub is sync; safe to call after drop.
            let access = match issue_jwt_with_google_sub(state, &sub, google_sub) {
                Ok(t) => t,
                Err(e) => return oauth_error(StatusCode::INTERNAL_SERVER_ERROR,
                                              &format!("JWT issuance failed: {e}")),
            };
            let body = serde_json::json!({
                "access_token": access,
                "token_type": "Bearer",
                "expires_in": jwt_ttl_secs(),
                "refresh_token": new_refresh.plaintext,
                "scope": "mcp",
            });
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => oauth_error(StatusCode::UNAUTHORIZED, &format!("invalid_grant: {e}")),
    }
}
```

**No `spawn_blocking` needed.** `grep -rn 'spawn_blocking' mcp/src`
returns zero hits — the codebase already runs SQLite work synchronously
inside short-locked scopes. SQLite calls are sub-millisecond on this
workload; the Tokio reactor stall is acceptable per the existing
`payment.rs`, `confirmation_token.rs`, `escrow.rs` precedent.

### §I.6 — Background evictor

**Precedent** (`mcp/src/confirmation_token.rs:259-267`) — quoted verbatim:

```rust
pub fn start_evictor(ledger: Arc<ConfirmationLedger>) -> tokio::task::JoinHandle<()> {
    let tick = ledger.evict_tick();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tick).await;
            ledger.evict_expired();
        }
    })
}
```

**Recommended new evictor** in `mcp/src/oauth/refresh.rs`:

```rust
pub fn start_refresh_evictor(
    store: Arc<Mutex<rusqlite::Connection>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let tick = std::time::Duration::from_secs(EVICTOR_TICK_SECS); // 3600s = 1h
        loop {
            tokio::time::sleep(tick).await;
            let result = {
                let conn = match store.lock() {
                    Ok(g) => g,
                    Err(_) => continue, // poisoned — skip this tick
                };
                evict_expired(&conn)
            };
            match result {
                Ok(n) if n > 0 => tracing::info!(target: "mnemonic_mcp::oauth::refresh",
                    "evicted {n} expired refresh-token rows"),
                Ok(_) => {}
                Err(e) => tracing::warn!(target: "mnemonic_mcp::oauth::refresh",
                    "refresh-token evictor failed: {e}"),
            }
        }
    })
}
```

**Spawn site** (`mcp/src/main.rs:635` neighbourhood):

```rust
let _confirmation_evictor = confirmation_token::start_evictor(confirmation_ledger);
let _refresh_evictor = oauth::refresh::start_refresh_evictor(store_arc.clone()); // NEW
```

`EVICTOR_TICK_SECS = 3600` matches D7 (hourly). The confirmation-token
evictor uses 60s because confirmation TTL is 5 min; refresh-token TTL
is 1 year, so a 1h sweep amortises perfectly.

### §I.7 — Discovery metadata update

**Current** (`mcp/src/oauth/mod.rs:1178-1191`):

```rust
pub async fn oauth_authorization_server_metadata() -> Response {
    let body = serde_json::json!({
        "issuer": SERVER_ORIGIN,
        "authorization_endpoint": format!("{SERVER_ORIGIN}/oauth/authorize"),
        "token_endpoint": format!("{SERVER_ORIGIN}/oauth/token"),
        "registration_endpoint": format!("{SERVER_ORIGIN}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": ["mcp"],
    });
    (StatusCode::OK, Json(body)).into_response()
}
```

**One-line change**:

```rust
"grant_types_supported": ["authorization_code", "refresh_token"],
```

Test (`mcp/src/oauth/mod.rs` `mod tests`):

```rust
#[tokio::test]
async fn test_discovery_advertises_refresh_token_grant() {
    let resp = oauth_authorization_server_metadata().await;
    let bytes = http_body_util::BodyExt::collect(resp.into_body())
        .await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let grants = body["grant_types_supported"].as_array().unwrap();
    assert!(grants.iter().any(|g| g == "refresh_token"));
    assert!(grants.iter().any(|g| g == "authorization_code")); // back-compat
}
```

### §I.8 — `TestServerBuilder` extension

**Current builder build()** (`mcp/tests/_helpers/mod.rs:104-126`):

```rust
pub fn build(self) -> TestServer {
    let state = mock_state_with(
        &self.storage_mode,
        &self.payment_mode,
        self.sign_memory_cost_micro_usdc,
    );
    let oauth_state = Arc::new(OAuthState::new(TEST_JWT_SECRET));
    let app = Router::new()
        .route("/mcp", post(mcp_handler))
        .route("/api/pending/{correlation_id}", get(get_pending_handler))
        .route("/api/sign-callback", post(sign_callback_handler))
        .layer(middleware::from_fn_with_state(
            oauth_state.clone(),
            oauth::bearer_auth_middleware,
        ))
        .with_state(state.clone());
    TestServer { state, oauth_state, app }
}
```

**New builder method + conditional mount**:

```rust
pub struct TestServerBuilder {
    storage_mode: String,
    payment_mode: String,
    sign_memory_cost_micro_usdc: i64,
    oauth_token: bool, // NEW
}

impl TestServerBuilder {
    pub fn with_oauth_token(mut self, enabled: bool) -> Self {
        self.oauth_token = enabled;
        self
    }

    pub fn build(self) -> TestServer {
        let state = mock_state_with(/* unchanged */);
        let oauth_state = Arc::new(OAuthState::new(TEST_JWT_SECRET));
        // ...

        let mut app = Router::new()
            .route("/mcp", post(mcp_handler))
            .route("/api/pending/{correlation_id}", get(get_pending_handler))
            .route("/api/sign-callback", post(sign_callback_handler))
            .layer(middleware::from_fn_with_state(
                oauth_state.clone(),
                oauth::bearer_auth_middleware,
            ))
            .with_state(state.clone());

        // NEW — mount /oauth/token on the SAME tower stack so a test can
        // hit it via `app.clone().oneshot(...)`. /oauth/token does not need
        // bearer_auth (URI-allowlisted in production), but adding the
        // middleware layer is harmless for tests.
        if self.oauth_token {
            let oauth_routes = Router::new()
                .route("/oauth/token", post(oauth::token_handler))
                .with_state(oauth_state.clone());
            app = app.merge(oauth_routes);
        }

        TestServer { state, oauth_state, app }
    }
}
```

`/mcp` is currently mounted at `_helpers/mod.rs:112-120` — copy that
shape for `/oauth/token` (different `with_state` because the handler
takes `State<Arc<OAuthState>>` not `State<Arc<McpState>>`).

**One subtlety** — the test harness's `OAuthState::new(TEST_JWT_SECRET)`
call lives at `_helpers/mod.rs:111`. After §I.4's change to widen the
signature, this builder must also build an in-memory SQLite connection
for `refresh_store` (use `rusqlite::Connection::open_in_memory()?` then
`migrate_refresh_tokens` on it). Test salt = `[0xAB; 32]`.

### §I.9 — Integration test layout

**File**: `mcp/tests/oauth_refresh_e2e.rs` (NEW)

Use the standard `mod _helpers;` pattern (already shared by
`modes_per_request.rs`). Each test builds a `TestServer` via
`TestServer::builder().with_oauth_token(true).build()`.

```rust
mod _helpers;
use _helpers::TestServer;

// AC1 — authorization_code grant returns refresh_token field.
#[tokio::test]
async fn test_authcode_grant_returns_refresh() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (code, verifier) = mint_authcode(&server).await;
    let resp = post_token(&server, json!({"code": code, "code_verifier": verifier})).await;
    assert!(resp["refresh_token"].is_string());
    assert!(resp["access_token"].is_string());
}

// AC2 — refresh_token grant rotates: new pair, old becomes revoked.
#[tokio::test]
async fn test_refresh_grant_returns_new_pair() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (_access, refresh) = bootstrap_oauth(&server).await;
    let resp = post_token(&server, json!({
        "grant_type": "refresh_token", "refresh_token": refresh.clone()
    })).await;
    let new_refresh = resp["refresh_token"].as_str().unwrap();
    assert_ne!(new_refresh, refresh);
    assert!(resp["access_token"].is_string());
}

// AC3 — old refresh presented outside 5s grace → 401 (after the grace
// has expired AND the family is not yet revoked, i.e. test sleeps 6s).
#[tokio::test]
async fn test_old_refresh_outside_reuse_interval_rejected() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (_a, refresh1) = bootstrap_oauth(&server).await;
    let _ = rotate(&server, &refresh1).await;
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    let resp_status = post_token_status(&server, json!({
        "grant_type": "refresh_token", "refresh_token": refresh1
    })).await;
    assert_eq!(resp_status, StatusCode::UNAUTHORIZED);
}

// AC4 — replay outside grace revokes the entire family. After replay,
// the NEW refresh token (from the legitimate rotation) is also rejected.
#[tokio::test]
async fn test_replay_outside_reuse_revokes_family() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (_a, r1) = bootstrap_oauth(&server).await;
    let r2 = rotate(&server, &r1).await;
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    let _ = post_token_status(&server, json!({
        "grant_type": "refresh_token", "refresh_token": r1
    })).await; // triggers family revoke
    let status = post_token_status(&server, json!({
        "grant_type": "refresh_token", "refresh_token": r2
    })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// AC5 — expired refresh → 401 WITHOUT revoking the family. Use a test
// helper that directly inserts a row with expires_at in the past.
#[tokio::test]
async fn test_expired_refresh_rejected_no_family_revoke() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (plaintext, family_id) = insert_expired_refresh_for_test(&server).await;
    let status = post_token_status(&server, json!({
        "grant_type": "refresh_token", "refresh_token": plaintext
    })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Sibling row in the same family must still be revoked=0.
    assert!(family_has_unrevoked_rows(&server, &family_id));
}

// AC6 — rotation extends expires_at by another 1y from now.
#[tokio::test]
async fn test_rolling_expires_at_extended() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (_a, r1) = bootstrap_oauth(&server).await;
    let pre_exp = read_expires_at_for(&server, &r1);
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let r2 = rotate(&server, &r1).await;
    let post_exp = read_expires_at_for(&server, &r2);
    assert!(post_exp >= pre_exp + 1, "rotation must roll expires_at forward");
}

// AC7 — discovery JSON advertises refresh_token grant.
#[tokio::test]
async fn test_discovery_advertises_refresh_grant() {
    let body = fetch_discovery(&TestServer::builder().build()).await;
    let grants = body["grant_types_supported"].as_array().unwrap();
    assert!(grants.iter().any(|g| g == "refresh_token"));
}

// AC8 — access token shape unchanged. Decode + assert claim set matches
// the existing Claims struct byte-for-byte; presence of refresh_token
// in the response must not perturb the JWT.
#[tokio::test]
async fn test_access_token_format_unchanged() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (access, _r) = bootstrap_oauth(&server).await;
    let claims = oauth::verify_jwt(&server.oauth_state, &access).unwrap();
    assert_eq!(claims.aud, "mcp");
    assert_eq!(claims.iss, "mcp.mnemonik.xyz");
}

// AC9 — anonymous recall path still works (refresh-token feature must
// not affect the allowlisted recall semantics).
#[tokio::test]
async fn test_anonymous_recall_unchanged() {
    let server = TestServer::builder().build(); // no /oauth/token mount
    let resp = server.call_tool(None, "mnemonic_recall", json!({"query":"x"})).await;
    assert!(resp.status.is_success());
}

// AC10 — content-type parity: form-encoded refresh grant works the same
// as JSON-encoded. Two sub-tests; assert identical wire output.
#[tokio::test]
async fn test_refresh_grant_content_type_parity() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (_a, r) = bootstrap_oauth(&server).await;
    let body_form = format!("grant_type=refresh_token&refresh_token={r}");
    let resp_form = post_token_raw(&server, "application/x-www-form-urlencoded",
                                    body_form.into_bytes()).await;
    assert_eq!(resp_form.status(), StatusCode::OK);
    // Build a SEPARATE server (the form path consumed the token) and
    // assert JSON path returns the same shape.
}

// AC11 — legacy client that omits grant_type AND refresh_token still
// works (default grant_type = authorization_code).
#[tokio::test]
async fn test_back_compat_ignores_refresh_field() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (code, verifier) = mint_authcode(&server).await;
    let resp = post_token(&server, json!({"code": code, "code_verifier": verifier})).await;
    assert!(resp["access_token"].is_string());
}

// AC12 — concurrent rotation within 5s grace is idempotent: 10 parallel
// requests with the SAME old refresh_token, all within reuse-interval,
// resolve to the SAME successor access_token / refresh_token pair.
#[tokio::test]
async fn test_concurrent_rotation_idempotent_within_reuse() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let (_a, r1) = bootstrap_oauth(&server).await;
    let futs = (0..10).map(|_| rotate(&server, &r1));
    let results = futures::future::join_all(futs).await;
    // All 10 must succeed and return the same refresh_token plaintext.
    let first = results[0].as_ref().unwrap().clone();
    for r in &results[1..] {
        assert_eq!(r.as_ref().unwrap(), &first);
    }
}

// AC13 — malformed refresh grant (missing refresh_token) → 400
// invalid_request, NOT 401.
#[tokio::test]
async fn test_malformed_refresh_grant_invalid_request() {
    let server = TestServer::builder().with_oauth_token(true).build();
    let status = post_token_status(&server, json!({
        "grant_type": "refresh_token"
        // refresh_token omitted
    })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
```

Helpers (`mod _helpers` extensions) to author:

- `bootstrap_oauth(&server) -> (access, refresh)` — runs the full
  authorize → token flow once, returning both tokens. Reuses the
  pattern from `oauth/mod.rs:1986-2030`.
- `rotate(&server, &refresh) -> String` — POSTs `grant_type=refresh_token`,
  returns the new refresh plaintext.
- `post_token`, `post_token_status`, `post_token_raw` — thin wrappers
  around `app.oneshot` mirroring `_helpers::TestServer::call_tool`.
- `insert_expired_refresh_for_test(&server) -> (plaintext, family_id)` —
  uses `server.state.store.lock()` to write a row with `expires_at = 0`
  bypassing the public mint API.

### §I.10 — Implementation risks

1. **Salt rotation strategy.** Changing `MCP_REFRESH_TOKEN_SALT`
   invalidates EVERY outstanding refresh token because
   `hex(blake3(new_salt || raw_bytes))` no longer matches the stored
   hash. This is the operational equivalent of "rotate the JWT secret":
   every active client must re-OAuth. Tech-spec should either document
   the salt as immutable-after-first-boot (recommended) or include a
   dual-hash window during a known rotation. Generating fresh-on-boot
   when env is absent (the `confirmation_token` precedent at
   `confirmation_token.rs:90-105`) is fine for dev but every restart
   invalidates refresh tokens — not acceptable for production.

2. **`token.json` cache file is out-of-scope (D15) BUT the read path
   may still need awareness.** `cache_minted_token`
   (`mcp/src/oauth/mod.rs:1101-1138`) writes ONLY the access token
   today:
   ```rust
   let token = mnemonic_core::identity::TokenJson {
       jwt: jwt.to_string(),
       expires_at,
       sub: sub.to_string(),
   };
   ```
   The cache will continue to be invalidated on every JWT expiry until a
   future feature wires refresh-token retrieval through it. The tech-spec
   author can leave this confidently alone — the V1 refresh flow lives
   entirely server-side and clients (Cursor, VS Code, Claude.ai) manage
   refresh tokens in their own credential stores via the OAuth response.

3. **`WWW-Authenticate` header on `/mcp` 401s unchanged.**
   `jsonrpc_unauthorized` at `mcp/src/oauth/mod.rs:1543-1570` always
   emits `Bearer realm="..." error="invalid_token" ...
   resource_metadata="..."`. The refresh-token feature does NOT touch
   this — refresh failures happen at `/oauth/token`, which returns
   `oauth_error(BAD_REQUEST | UNAUTHORIZED, "...")` (`mod.rs:1158`)
   with no WWW-Authenticate header. RFC-compliant — WWW-Authenticate
   is for the protected resource, not for the token endpoint.

4. **`/oauth/*` tower-governor rate-limit applies unchanged.**
   `mcp/src/main.rs:810-829`: 5 burst + 1 req/s refill per IP. The
   refresh grant adds traffic at the rate of one POST per access-token
   expiry per client ≈ 1/hour. Per-IP is fine for that volume; CI runs
   with many concurrent clients should set `OAUTH_RATELIMIT_DISABLE=1`
   (already documented at `main.rs:814-830`). No new rate-limit wiring
   needed.

5. **Existing oauth unit tests — extend, don't shadow.**
   `mod tests` at `mcp/src/oauth/mod.rs:1572-end` contains
   `test_token_valid_verifier_returns_jwt` (`:1982`),
   `test_token_invalid_verifier_401` (`:2033`),
   `test_token_expired_code_60s_401` (`:2080`) and ~15 more. The
   refresh-grant tests should live in the SAME `mod tests` (one
   `#[tokio::test]` per AC) and reuse the existing `fresh_state`,
   `build_authorize_router`, `post_json` helpers. The
   `_helpers::TestServer` integration suite covers the cross-handler
   wiring (refresh-grant → JWT-protected `/mcp` call); unit tests
   cover the rotation function and dispatch logic in isolation. Do
   not shadow — extend.

6. **`fresh_state()` upgrade hazard.** Every unit test in
   `oauth/mod.rs::mod tests` calls `fresh_state()` (`:1592`). After
   §I.4's signature change, this helper must build an in-memory
   `Connection` + run `migrate_refresh_tokens` before returning the
   `OAuthState`. Existing tests that DON'T touch the refresh table
   still work because the migration is idempotent and the helper now
   does the extra work transparently. Worth a single explanatory
   comment so future authors don't strip it as "unused".

7. **5-second reuse-interval test flake risk.** AC3, AC4, and AC5 use
   `tokio::time::sleep` with real wall-clock — slow CI may push the
   6-second waits up. Two mitigations:
   (a) Pull `REUSE_INTERVAL_SECS` into a `&OAuthState` field instead
   of a module-level const so tests can lower it to 100ms.
   (b) Use `tokio::time::pause()` + `advance()` to drive deterministic
   virtual time. Pattern unused in this codebase today; precedent worth
   adding for this feature.
   Recommendation: (a) — same shape as `ConfirmationLedger::with_config`
   at `confirmation_token.rs:96-105` which already exposes `ttl` and
   `evict_tick` as fields for the same reason.


