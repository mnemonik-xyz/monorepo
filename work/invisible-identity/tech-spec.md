---
created: 2026-05-21
backfilled: 2026-05-22  # post-implementation backfill to match Waves 1-4 reality + audit fixes (43a6696)
status: frozen-implementation  # Waves 1-4 done; Wave 5 T15 blocked_on_user, T19 pre-deploy QA in_progress
branch: feat/invisible-identity
size: L  # re-sized in user-spec round 1; absorbs work/keypair-sync/, 4 surfaces, 19 tasks, marketplace-critical
absorbs: work/keypair-sync/
---

# Tech Spec: Invisible Identity — silent bootstrap + cross-surface sync

## Solution

Two-layer delivery in one feature shipment.

**Layer 1 — Silent bootstrap.** Every Mnemonic surface that needs an Ed25519 identity (Node CLI `@mnemonik-xyz/cli`, Rust stdio MCP server `mnemonic-mcp`) routes through a single internal `identity.ensure()` (TS) / `identity::ensure()` (Rust) on startup. Behavior is identical across both languages and both file/keychain paths:

1. If `~/.mnemonic/identity.json` exists and contains `keychain_ref` → read pubkey from file, secret from OS keychain. Done.
2. Else if it exists and contains `secret` (legacy) → migrate to keychain when available, leave on disk when not. Same pubkey throughout.
3. Else → generate new Ed25519 keypair, write to keychain + stub file. Emit a single line on stderr: `mnemonic: identity created did:sol:... stored in OS keychain`.
4. Cross-language interop comes for free because both sides read/write the same keychain entry (`service=xyz.mnemonik.identity`, `account=default`) using the same inner JSON shape that lives in legacy file-fallback mode.

**Layer 2 — Cross-surface sync.** Absorbs the entirety of `work/keypair-sync/`:

5. `mnemonic identity status` — drift detector comparing CLI identity vs cached JWT.sub from `~/.mnemonic/token.json`.
6. Webapp `/install` page emits IDE-specific deeplinks with a baked `Authorization: Bearer <jwt>` header. Cursor / VS Code / Claude Desktop receive a config that's already authenticated to the webapp's keypair — structural drift between webapp localStorage and IDE-side JWT becomes impossible.
7. Server endpoints `/api/cli-bootstrap/issue` / `/issue-from-cli` / `/redeem` (existing for the first; new for the second) form a symmetric ticket protocol for explicit Send-to-CLI and Push-to-webapp flows. Tickets carry only x25519-wrapped secrets, TTL 5 min, single-use.
8. Webapp "Generate new keypair" modal gains drift-warning options (Send to CLI / Download backup / Cancel) before destructive replacement.

Architecturally there is no new service. Everything lands as:
- 2 new modules in `mnemonic-core` (`identity/ensure.rs`, `identity/keystore.rs` with 3 impls)
- 1 new wire-up in `mcp/src/main.rs`
- 2 new modules in `@mnemonik-xyz/cli` (`src/identity/ensure.ts`, `src/identity/keystore.ts`)
- 1 new dep in each language (`@napi-rs/keyring` on Node, `keyring` crate on Rust)
- 1 new CLI command (`mnemonic identity` with `status` / `pull-from-webapp` / `push-to-webapp` subcommands)
- 1 new server endpoint (`/api/cli-bootstrap/issue-from-cli`) + 1 modified (`/redeem` accepts both ticket origins)
- 1 modified webapp page (`/install` deeplink generator) + 1 modified component (IdentityPanel drift-warning modal)
- `work/keypair-sync/` archived to `work/completed/keypair-sync/MOVED.md`

## Architecture

### What we're building / modifying

**`mnemonic-core` (Rust library) — new files:**

- `core/src/identity/ensure.rs` (new) — `pub fn ensure() -> anyhow::Result<Identity>`. Pure function, no I/O abstraction yet; pulls disk + keychain via `KeyStore` trait.
- `core/src/identity/keystore.rs` (new) — `pub trait KeyStore` with `get`, `set`, `remove`, `available`. Plus three impls.
- `core/src/identity/keystore_os.rs` (new) — `OsKeyStore` wrapping the `keyring` crate. macOS Keychain, Linux Secret Service, Windows Credential Manager.
- `core/src/identity/keystore_file.rs` (new) — `FileKeyStore` operating on `~/.mnemonic/identity.json` in legacy format `{secret, pubkey_base58}`.
- `core/src/identity/keystore_memory.rs` (new, `#[cfg(test)]` only) — `MemoryKeyStore` for unit tests.
- `core/src/identity/mod.rs` (modified) — re-export new modules; the existing `load_or_create_keypair(path)` is **removed** in the same PR that introduces `ensure()` (we are pre-1.0, no external Rust consumers documented).

**`mnemonic-mcp` (Rust binary) — modified:**

- `mcp/src/main.rs` — call `mnemonic_core::identity::ensure()` after argv parsing, before transport start. Replaces the existing direct call to `load_or_create_keypair`. Resulting `Identity` is plumbed into `McpState` exactly as today.

**`@mnemonik-xyz/sdk` (Node SDK) — modified:**

- `packages/sdk/src/identity/keypair.ts` — `Keypair.fromJSON()` learns to parse both legacy (`{secret, pubkey_base58}`) and stub (`{pubkey_base58, did_sol, keychain_ref, created_at}`) shapes. When stub-shaped, throws a typed error `IdentityRequiresKeystore` carrying the `keychain_ref`; CLI catches and resolves.

**`@mnemonik-xyz/cli` (Node CLI) — new files + modified entrypoint:**

- `packages/cli/src/identity/ensure.ts` (new) — Node equivalent of `identity::ensure()`. Same algorithm, same disk layout, same keychain entry coordinates.
- `packages/cli/src/identity/keystore.ts` (new) — TS `KeyStore` interface + `OsKeyStore` (uses `@napi-rs/keyring`) + `FileKeyStore` + `MemoryKeyStore` for tests.
- `packages/cli/src/commands/identity.ts` (new) — implements three subcommands:
  - `mnemonic identity status` — drift detector
  - `mnemonic identity pull-from-webapp <ticket>` — redeem Send-to-CLI ticket
  - `mnemonic identity push-to-webapp` — issue ticket, print QR + short code
- `packages/cli/src/index.ts` (modified) — entry shim calls `await ensure()` before dispatching to any command except `--help` / `--version` / `identity status` itself (status must show drift even if migration is the next user action).

**`mnemonic-mcp` server endpoints — webapp ↔ CLI ticket flow:**

- `mcp/src/api.rs` — extends the existing `/api/cli-bootstrap/{issue,redeem}` pair (created by `work/mnemonic-cli/` Deviation 2) with a third endpoint `/api/cli-bootstrap/issue-from-cli`. Same `BootstrapTickets` LRU/TTL store. Differences explained under Decisions.

**Webapp (`webapp/`) — modified:**

- `webapp/src/pages/install.tsx` — generates `mcp.json` config with `Authorization: Bearer <jwt>` header baked in for the logged-in user. Adds platform-detection + deeplink scheme buttons (`cursor://`, `vscode:`, manual for Claude Desktop).
- `webapp/src/components/IdentityPanel.tsx` (or equivalent) — drift-warning modal in "Generate new keypair" path.

**Archival:**

- `work/keypair-sync/user-spec.md` → `work/completed/keypair-sync/user-spec.md` (preserved verbatim)
- `work/completed/keypair-sync/MOVED.md` (new) — one-line pointer to `work/invisible-identity/`

### How it works

**Bootstrap call path (Node CLI):**

```
bin/mnemonic.ts
  → src/index.ts (cli.parse)
    → if (cmd !== '--help' && cmd !== '--version' && cmd !== 'identity status')
        await identity.ensure()
            → fileStore.read()
               ├─ ENOENT      → generate keypair → keychain.set() → fileStore.writeStub()
               ├─ {secret,..} → legacy: keychain available? migrate; else keep legacy
               └─ {pubkey,..} → keychain.get(keychain_ref) → return Identity
    → cli.dispatch(cmd)
```

**Bootstrap call path (Rust mcp-server):**

```
mcp/src/main.rs
  → tracing::init
  → config::load_env
  → let identity = mnemonic_core::identity::ensure(KeyStoreSelection::Auto)?;
  → let state = McpState::new(identity, ...);
  → transport::start(state).await
```

Same algorithm, byte-equal disk format, byte-equal keychain entry — so order of first use (CLI-first vs server-first) doesn't matter.

**Sync ticket flow (webapp → CLI, Send-to-CLI):**

```
Webapp: POST /api/cli-bootstrap/issue
        body: {wrapped_secret, eph_pub}    # wrap = x25519(server_pub) → stored only ciphertext
        ← {ticket_id, short_code, expires_at}
        UI: display QR(ticket_id) + short_code
CLI:    mnemonic identity pull-from-webapp ABCD-1234
        → POST /api/cli-bootstrap/redeem  body: {short_code, cli_pub}
        ← {wrapped_secret, eph_pub}        # server re-wraps to cli_pub before returning
        → x25519_decrypt(wrapped_secret, cli_priv) → secret_bytes
        → keystore.set(secret_bytes)
        → fileStore.writeStub(pubkey_base58)
```

**Sync ticket flow (CLI → webapp, Push-to-webapp):**

```
CLI:    mnemonic identity push-to-webapp
        → POST /api/cli-bootstrap/issue-from-cli   body: {wrapped_secret_to_server, eph_pub}
        ← {ticket_id, short_code, expires_at}
        → print short_code + QR(URL https://mnemonik.xyz/install?pull=<ticket_id>)
Webapp: user opens URL on logged-in browser
        → POST /api/cli-bootstrap/redeem  body: {short_code, web_pub}
        ← {wrapped_secret, eph_pub}
        → decrypt in browser → localStorage.setItem('mnemonic_keypair', JSON.stringify({secret, pubkey_base58}))
```

Both directions use the same `/redeem` endpoint. The server tracks ticket origin (`from=webapp` vs `from=cli`) and re-wraps to the redeemer's pubkey on read.

### Shared resources

- **OS keychain entry** at `service=xyz.mnemonik.identity`, `account=default`. Single entry per machine. Both Node CLI and Rust mcp-server read/write it. Concurrent access is serialized by the OS — `keyring` crate and `@napi-rs/keyring` both block briefly on platform-level mutex; no application-level locking required.
- **`~/.mnemonic/identity.json`** — single file, two possible shapes (stub or legacy). Both Node and Rust must round-trip both shapes byte-identically. Mode 0600 on Unix; Windows ACL "Owner only" via existing `fs.utimes`/equivalent on the Node side (already handled).
- **`~/.mnemonic/token.json`** — unchanged from current. Only read by `identity status` for drift detection.
- **`BootstrapTickets` in-memory store** in `mcp/src/api.rs` — LRU 1000 entries, TTL 5min. Already introduced by `work/mnemonic-cli/`. Extended (not replaced) to support the `from-cli` direction.

## Decisions

### Decision 1: Keychain coordinates `service=xyz.mnemonik.identity`, `account=default`
**Decision:** Fixed strings, hardcoded in both Rust and Node sides. No env-var override in Phase 1 (would defeat cross-language interop).
**Rationale:** Reverse-DNS service name follows Apple Keychain convention and works on Secret Service (D-Bus collection name) and Windows Credential Manager (target name) identically. `default` account leaves room for future multi-profile (`work` / `personal`) without breaking the current single-entry layout. Supports user-spec acceptance criterion: "OS keychain entry created and read by both language sides." `[TECHNICAL]`
**Alternatives considered:** Per-version namespace (`xyz.mnemonik.identity.v1`) — rejected because migrations across versions become painful and no version bump is foreseen. Env-var override — backlog; needed only when we add multi-profile.

### Decision 2: `did:sol:` remains the default DID format
**Decision:** No change to current behavior. `did:sol:<base58_pubkey>` is what `mnemonic whoami` prints, what attestations carry, and what the webapp displays.
**Rationale:** User-spec decision B explicitly preserves `did:sol:`. Switching to `did:key:` would force every existing attestation reference to be re-rendered, every `local:` synthetic tx to be re-anchored conceptually, and every UI to relearn the prefix. `did:key` may be revisited if/when we add a hardware-wallet signer that doesn't have a Solana pubkey nature, but not in this feature. `[USER]`
**Alternatives considered:** Switch to `did:key:` for cryptographic generality — rejected. Support both via `--did-format` flag — backlog (one-line opt-in is trivial, but no current consumer needs it).

### Decision 3: Keychain access is lazy and silent at bootstrap
**Decision:** `KeyStore::get` is called only when a private key is actually needed (i.e. when a command signs or proves). `identity.ensure()` only verifies that the entry **exists** (pubkey readable from file, keychain_ref valid), it does NOT pull the secret. As a result, the OS will not prompt for keychain unlock when the user runs e.g. `mnemonic whoami` — only when they run `mnemonic sign`.
**Rationale:** Marketplace "no setup prompts on first run" gate covers OS-level prompts too. Lazy access also reduces the prompt surface to commands that actually need to sign. On macOS the first signing prompt offers "Always Allow" which converts the lifetime cost to one-time. `[TECHNICAL]`
**Alternatives considered:** Pull secret at bootstrap, cache in memory for process lifetime — rejected because (a) it prompts on every command even `--help`, (b) keeping the secret in memory longer is a security regression. Cache the secret in memory only after first sign in a process — current design implicitly does this via reused `Identity` struct.

### Decision 4: File-fallback triggers on any keychain error, not just unavailability
**Decision:** `OsKeyStore::set` and `OsKeyStore::get` are wrapped in a probe. If the probe returns `KeystoreError::PlatformUnavailable` (no D-Bus, no Credential Manager, no SecKeychain) OR `KeystoreError::Locked` after a single retry OR any unexpected `Error` — `identity.ensure()` falls back to `FileKeyStore` and emits a single stderr line `mnemonic: OS keychain unavailable (<reason>), using ~/.mnemonic/identity.json directly`. The reason is short and non-sensitive (no key material in logs).
**Rationale:** Headless Linux without D-Bus is the common case (Docker, CI, SSH session without forwarded keyring). Treating "locked keychain on a desktop OS where it should work" as a hard error would create unreachable scenarios — better to print the cause once and proceed. The bigger risk is the opposite: silently writing to the file when keychain "works" but in a broken way, leading to drift between Node and Rust later. Mitigated by the cross-language interop test matrix (Wave 3). `[TECHNICAL]`
**Alternatives considered:** Hard-fail on any keychain error — rejected, breaks headless. Silently fall back without stderr line — rejected, hides drift sources. Retry forever — rejected, hangs CI.

### Decision 5: Disk layout — `identity.json` (stub or legacy) + `README.txt`, no separate `.pub` file
**Decision:** Two valid file shapes at the same path:
- **Stub (keychain-backed):** `{"pubkey_base58":"...","did_sol":"did:sol:...","keychain_ref":"xyz.mnemonik.identity/default","created_at":"2026-05-21T10:23:45Z"}`
- **Legacy (file-fallback):** `{"secret":[...64 bytes...],"pubkey_base58":"..."}` — unchanged from today

Detection: parse as JSON, branch on presence of `"secret"` key. Both Node and Rust use this exact rule. `~/.mnemonic/README.txt` is written on first creation only, contains one paragraph explaining the directory layout and where to find docs. `[TECHNICAL]`
**Rationale:** Single file at a single path is the least surprise. Separate `.pub` file would force consumers to know about both. Legacy shape stays bit-identical so older Mnemonic versions (pre-this-feature) can still read the file if a user downgrades. `[USER]`
**Alternatives considered:** Move to `identity.cose` (binary COSE_Key format) — rejected per user-spec decision B (no breaking format change). Separate `identity.pub` for pubkey + secret in keychain only — rejected because legacy-format users on file-fallback need pubkey + secret in one file anyway.

### Decision 6: Migration is in-place, idempotent, no `.bak` rename
**Decision:** On legacy → stub migration, the file at `~/.mnemonic/identity.json` is rewritten in-place. No `.bak` copy is left behind. Same path, same filename, only inner shape changes. If the keychain `set` succeeds but the file rewrite fails (rare — disk full, permissions), the keychain entry is removed before propagating the error, leaving the legacy file intact. Migration runs at most once per `ensure()` call; if the file is already in stub shape, no migration is attempted.
**Rationale:** A `.bak` file would leak the secret to a second location on disk indefinitely (until the user manually deletes it) — security regression vs. today. In-place atomic rewrite via tempfile + rename eliminates the partial-write window. Rollback via "delete keychain entry on file-write failure" guarantees the user can re-run after fixing whatever blocked the rewrite. `[SECURITY]`
**Alternatives considered:** Leave `.bak` for user safety — rejected (security). Two-phase commit with explicit user confirmation — rejected (defeats invisible-bootstrap goal).

### Decision 7: Stderr behavior — CLI emits one line, MCP-server stays silent
**Decision:** Per-surface split (ratified by audit fix `43a6696` after wave 5 review):

**Node CLI (`@mnemonik-xyz/cli`)** — first-time identity creation emits exactly one line to stderr:
- `mnemonic: identity created did:sol:H8x4...c4v stored in OS keychain` (success)
- `mnemonic: identity created did:sol:H8x4...c4v stored in ~/.mnemonic/identity.json (OS keychain unavailable: <reason>)` (file-fallback)
- `mnemonic: legacy identity migrated to OS keychain` (migration path)

Subsequent runs print nothing. The `--quiet` CLI flag suppresses even the first-creation line. The `--json` flag formats the creation event as one stderr JSON object.

**Rust mcp-server (`mnemonic-mcp`)** — completely silent on stderr at any point of bootstrap. Identity creation, migration, fallback are all observable only via structured `tracing::info!` / `tracing::warn!` records routed to whatever subscriber the operator wired up (default: stdout-bound for stdio transport — see "MCP stdio convention" below).

**`MNEMONIC_QUIET=1` env var — stable user-facing contract on BOTH surfaces.** Suppresses any startup-creation announcement (the CLI stderr line above; on the Rust mcp-server it additionally drops `tracing::info!("identity created ...")` records below the `warn` level). Needed because IDEs spawn stdio MCP without argv-flag plumbing and any unsolicited stderr breaks Cursor/Claude Desktop JSON-RPC framing. Stable across minor versions. `[USER]`

**MCP stdio convention.** When `--transport stdio`, the Rust mcp-server's `tracing` subscriber is wired to stdout (NOT stderr) because Cursor/Claude Desktop treat stderr as fatal error stream. Operators bringing up the HTTP transport may rebind to stderr via their own subscriber config — out of scope for this feature.

**Rationale:** Marketplace gate prohibits noisy startup, and unsolicited stderr from a stdio MCP server is interpreted as a crash signal by Cursor. One stderr line on the CLI is the minimum a human user needs to know an identity was created on their behalf (and where it lives, for backup purposes). The MCP server has no human user — its operator reads logs, not stderr.
**Alternatives considered:** Silent on CLI too — rejected, user has no clue an identity was created. Multi-line summary — rejected, too noisy. Stdout — rejected, breaks `mnemonic sign "x" | jq ...` pipes. Letting the MCP server keep the same stderr contract as CLI — rejected after `43a6696` when Cursor/Claude Desktop framing failures showed up in T15 smoke.

### Decision 8: No new Rust user CLI; extend Node `@mnemonik-xyz/cli` only
**Decision:** The user-facing CLI surface stays at `@mnemonik-xyz/cli` (Node). The Rust `mnemonic-mcp` binary gets `identity::ensure()` for its own startup but exposes no new user-facing subcommands. The `mnemonic identity status / pull-from-webapp / push-to-webapp` commands live in the Node CLI.
**Rationale:** User-spec decision A: do not fork the user CLI into two implementations. The Rust mcp-server is consumed by IDEs over stdio/HTTP — it has no human user surface. Adding a Rust user-CLI would mean two argv parsers, two help systems, two config-file layouts. Keep the Node CLI as the single human-facing tool. `[USER]`
**Alternatives considered:** Ship a Rust user CLI alongside the Node one for users who don't want a Node install — rejected per user-spec. Make the Rust mcp-server also a user CLI via flag-dispatch — rejected, conflates server lifecycle with one-shot commands.

### Decision 9: Node keychain library = `@napi-rs/keyring`
**Decision:** `@napi-rs/keyring` (v1.x). Hard dependency in `packages/cli/package.json`. Falls under devDependencies-free runtime dependencies. N-API native module, prebuilt binaries for darwin-arm64, darwin-x64, linux-x64-gnu, linux-arm64-gnu, win32-x64-msvc — covers the matrix.
**Rationale:** `keytar` (the obvious historical choice) is in maintenance mode and the underlying Node N-API contract is brittle in Node 22+. `@napi-rs/keyring` is actively maintained, depends only on the Rust `keyring` crate under the hood (same library the Rust side will use — implementation parity for free), and ships precompiled binaries via `napi-rs` infrastructure. `[TECHNICAL]`
**Alternatives considered:** `keytar` — rejected, maintenance mode. `node-keytar` — same package, same problem. Shell out to `security` (macOS) / `secret-tool` (Linux) / `cmdkey` (Windows) — rejected, three command surfaces to maintain and parse. Inline FFI via `koffi` — rejected, hand-rolling what `@napi-rs/keyring` already gives us.

### Decision 10: Rust keychain library = `keyring` crate
**Decision:** `keyring = "2"` (or current major). Hard dependency in `core/Cargo.toml`. No optional feature gate — keychain support is core functionality, not opt-in.
**Rationale:** De facto standard for cross-platform keychain in Rust. Same crate `@napi-rs/keyring` wraps, so format compatibility is automatic. Active maintenance, ships clean cross-compilation. `[TECHNICAL]`
**Alternatives considered:** `security-framework` (macOS only) + `secret-service-rs` + `winapi` direct — rejected, three crates to maintain. Vendored binding — over-engineering for a stable abstraction.

### Decision 11: Drift-detector reads only local state
**Decision:** `mnemonic identity status` reads (a) the local Identity via `KeyStore`, (b) `~/.mnemonic/token.json` if present, decoded JWT.sub. It does NOT make a network request to the webapp or any server endpoint to discover remote state. If the user wants to verify webapp state from the CLI, they run `mnemonic identity push-to-webapp` (which is an explicit user action).
**Rationale:** Network calls in a status command introduce latency, failure modes (offline, server down, JWT expired), and an implicit endpoint dependency. The drift cases the user-spec lists all manifest in local state: a JWT signed by one keypair sitting next to a different local identity. Local-only status catches all of them without going off-machine. `[TECHNICAL]`
**Alternatives considered:** Ping a `/api/whoami` endpoint to compare server-side pubkey — rejected, requires network and JWT, and the comparison is the same one we'd derive from local JWT.sub anyway. Watch webapp localStorage via a shared file — rejected, browsers don't expose localStorage to the OS filesystem.

### Decision 12: Ticket protocol = x25519-wrapped secret, single-use, 5min TTL
**Decision:** Ticket payload structure (in-memory on server, in the `BootstrapTickets` map):

```rust
struct Ticket {
    id: Uuid,                   // ticket_id
    short_code: String,         // e.g. "ABCD-1234", 8 chars from a 32-char alphabet (excl. confusables)
    origin: TicketOrigin,       // Webapp | Cli
    wrapped_secret: Vec<u8>,    // x25519_seal(secret, recipient_ephemeral_pub)
    eph_pub: [u8; 32],          // ephemeral x25519 pubkey used by the issuer to wrap
    expires_at: SystemTime,     // now + 5min
    redeemed: bool,             // single-use guard
    issuer_pubkey: [u8; 32],    // Ed25519 pubkey of the issuer (audit trail, not used for verification yet)
}
```

Redemption flow: redeemer presents short_code + their own ephemeral x25519 pub. Server re-wraps the secret to the redeemer's pubkey (decrypts using server-side ephemeral private, re-encrypts using redeemer's pub) and returns. The server never sees the raw long-lived secret in plaintext at rest — only in process memory during the unwrap-rewrap step.

Actually — refined: the wrap happens **client-to-client** with the server holding only the cryptotext. The issuer wraps the secret to the server's well-known x25519 pubkey (published on `/api/cli-bootstrap/server-pub`). The server unwraps once on redemption, re-wraps to the redeemer's submitted ephemeral pub, returns. Server-side plaintext exposure window: milliseconds during the rewrap call.
**Rationale:** Sending raw secret over the network is unacceptable. Wrapping to the redeemer's pubkey would require the redeemer to be known at issue time — which contradicts "issue, then redeem from another device". Server-as-broker with momentary unwrap is the minimum-trust model that still permits one-shot tickets. `[SECURITY]`
**Alternatives considered:** Pre-shared symmetric key via QR — possible but breaks the "open URL in another browser" UX. Webapp-managed PSI — over-engineered.

### Decision 13: Keychain entry inner format = legacy JSON, byte-equal across languages
**Decision:** The bytes stored in the keychain entry are exactly:

```json
{"secret":[1,2,...,64],"pubkey_base58":"H8x..."}
```

— UTF-8, no whitespace, key order `secret` then `pubkey_base58`. Same shape as today's file-fallback. Both Node and Rust serialize/deserialize with this exact ordering. JSON canonicalization is enforced via a shared golden test (Wave 3).
**Rationale:** Coordinating a new format across two languages is a recurring maintenance cost. Using the existing format costs nothing and means the file-fallback and keychain-backed paths exchange bytes through a single shape. Any future format change requires explicit cross-language migration. `[TECHNICAL]`
**Alternatives considered:** Store only the secret as raw 64 bytes — rejected, then Node/Rust must also coordinate on what "this is" later. CBOR / COSE_Key — rejected (user-spec decision B, no breaking change).

### Decision 14: `work/keypair-sync/` archived to `work/completed/keypair-sync/` at ship time
**Decision:** When this feature ships, perform `git mv work/keypair-sync work/completed/keypair-sync` (preserves history) and create `work/completed/keypair-sync/MOVED.md` with a single sentence: `Content absorbed into work/invisible-identity/. See its user-spec.md and tech-spec.md.` The original `user-spec.md` inside is preserved verbatim — anyone following an old reference still finds the text.
**Rationale:** A live `work/keypair-sync/` would confuse maintainers about which spec is authoritative. Deletion would lose history. Archival with a redirect is the standard pattern in this repo (compare `work/completed/`). `[PROCESS]`
**Alternatives considered:** Delete after merge — rejected, loses git-blame trail across the move. Keep both in parallel — rejected, ambiguous source of truth.

### Decision 15: JWT-baked deeplinks for Cursor / VS Code / Claude Desktop
**Decision:** `webapp/src/pages/install.tsx` generates a `mcp.json` config snippet with `headers: {Authorization: "Bearer <jwt>"}` baked in. Cursor uses `cursor://mcp/install?config=<base64-json>` deeplink scheme; VS Code uses `vscode:mcp/install?config=...`; Claude Desktop has no install deeplink — show a "Copy config to ~/Library/Application Support/Claude/claude_desktop_config.json" instruction with the baked JWT included. The JWT used is the current webapp session's JWT (read from localStorage). Webapp warns "this config contains a secret token, don't share it" above the button.
**Rationale:** Decision 5/7 of the absorbed keypair-sync user-spec — closes the IDE-side drift case structurally. JWT.sub equals webapp localStorage pubkey, IDE-side `/mcp` calls authenticate as the webapp user, no `mnemonic login` step needed. The chicken-and-egg constraint: Cursor/VS Code/Claude.ai DO support OAuth 2.1+PKCE against `mcp.mnemonik.xyz/oauth/*` for ongoing auth — but that handshake happens AFTER the MCP client is already instantiated in the IDE. At the "1-click install from webapp" moment the IDE has no JWT to start the OAuth dance, so pre-baked JWT bridges the install moment. After install, the OAuth-refresh path takes over without further user action. `[USER]`
**Alternatives considered:** Force OAuth-in-IDE at install time — requires Cursor/VS Code to launch a browser handshake before applying the MCP config, which neither does. One-shot bootstrap-nonce → OAuth-in-IDE — adds a user-visible interactive step that breaks marketplace "no required setup" guideline. Long-lived bootstrap tickets — rejected, JWT TTL of 1h is shorter and safer than a separate ticket type.

### Decision 16: Concurrent bootstrap is race-safe via atomic-rename + idempotent set
**Decision:** Two concurrent `ensure()` calls on the same machine (e.g. IDE simultaneously spawning stdio-MCP and a CLI hook on a fresh `~/.mnemonic/`) must not produce split-brain identity. Mechanism:

1. **Disk stub** written via `tempfile::NamedTempFile::new_in(parent) + sync_all() + persist(target)` — atomic-rename at the POSIX layer. The OS guarantees that `~/.mnemonic/identity.json` either points at the old inode or the new one, never a half-written file.
2. **Keychain set** is idempotent — `OsKeyStore::set(service, account, value)` overwrites any existing entry. Two writers racing produce the entry one of them wrote last (whichever wins the OS-level mutex inside `keyring`/`@napi-rs/keyring`).
3. **Reader path** verifies after open: stub's `pubkey_base58` must equal `derive_pubkey(keychain.get(keychain_ref).secret)`. Mismatch → loud error (see Decision 17 case c).

The winning writer's identity becomes the canonical one; the losing writer retries the read path on its next call (or on the same call if `set` raised "already-exists" + we treat that as success). Both surfaces end up with the same pubkey because the keychain entry — single OS-level resource — is the tiebreaker.

**Rationale:** First-run + concurrent-spawn is the high-risk callsite (IDE starts stdio-MCP and CLI-bound hook simultaneously on a fresh machine). Without atomic-rename, the disk stub could be left as a half-written file; without the keychain tiebreaker, the two writers could generate two different keys and stamp them on different surfaces. The race window is real — measured at sub-second on macOS during T15 smoke. `[TECHNICAL]`
**Alternatives considered:** Flock on `~/.mnemonic/.lock` — adds a file we have to clean up on crash, and `flock` semantics differ across platforms (Windows has no flock; would force a polyfill). Application-level mutex via a sidecar process — way over-engineered.

### Decision 17: Partial-state recovery is explicit, mismatch is loud
**Decision:** Three reachable partial states, each with a defined behavior. NO silent picking of one of the two sides.

| State | Cause | Behavior |
|-------|-------|----------|
| (a) stub file exists, keychain entry missing | User wiped keychain via Keychain Access; Linux Secret Service per-session loss; Docker volume mount lost in restart | Throw typed `IdentityRequiresKeystore { pubkey_base58, keychain_ref }`. CLI catches and prints actionable hint: `run 'mnemonic identity pull-from-webapp <code>' to restore`. Exit 1. |
| (b) keychain entry exists, stub file missing | User wiped `~/.mnemonic/` but not OS keychain | Silent rebuild: derive pubkey from keychain secret, write a fresh stub pointing at the existing keychain entry. No stderr line — this is the "recover gracefully" path. |
| (c) both exist BUT stub.pubkey ≠ derive_pubkey(keychain.secret) | Race in Decision 16's reader window; legacy migration bug; manual file-edit by user | Loud error: print `mnemonic: identity integrity mismatch — stub pubkey <X> does not match keychain-derived pubkey <Y>` to stderr, exit 3. Do NOT pick either side. User must run `mnemonic identity reset` (out of scope for this feature; backlog) or manually edit + retry. |

**Rationale:** The mismatch case (c) is a safety property — silently picking one side hides the corruption and could re-introduce a drift bug class identical to the one this feature was meant to eliminate. Cases (a) and (b) are common enough on real desktops that they need first-class handling, not "undefined behavior". `[TECHNICAL] [SECURITY]`
**Alternatives considered:** Always recover by regenerating from keychain — rejected, masks corruption. Prompt user interactively — rejected, breaks invisible-bootstrap. Self-heal by deleting both and re-creating — rejected, destroys data.

### Decision 18: Server-side ticket keypair lifetime = process
**Decision:** The `mcp/src/main.rs` startup generates a fresh `crypto_box::SecretKey` for the server's wrap-broker role and serves the matching `PublicKey` from `GET /api/cli-bootstrap/server-pub`. The keypair lives for the process lifetime — restart drops it. All in-flight tickets (TTL 5min window) become unredeemable on restart because the issuer-side `wrapped_secret` was sealed to the now-dropped server pubkey.

**Rationale:** Persisting the server's wrap-broker key to disk would create a permanent target for offline compromise (an attacker who gets the file can decrypt every historical ticket ciphertext they captured). Process-lifetime keys mean compromise requires live process access. The user-visible cost is a worst-case 5-minute window after restart where in-flight tickets fail with `ticket expired (or server restarted)` and the user retries — acceptable. `[SECURITY]`
**Alternatives considered:** Persist server key in OS keychain on the mcp host — rejected, mcp host doesn't have a per-deploy keychain abstraction. Rotate server key every 5min on a timer — rejected, adds complexity for no security gain over process-lifetime. Skip wrap-broker entirely (client-to-client crypto) — rejected, requires issuer and redeemer to know each other's pubkeys at issue time, which contradicts the "issue, then redeem from another device" UX.

## Data Models

### On-disk: `~/.mnemonic/identity.json`

**Stub format (keychain-backed):**

```json
{
  "pubkey_base58": "H8x4F2dCkP7zNcWqGmLpRtVxYbAhJoEsTuWnZQfXc4v",
  "did_sol": "did:sol:H8x4F2dCkP7zNcWqGmLpRtVxYbAhJoEsTuWnZQfXc4v",
  "keychain_ref": "xyz.mnemonik.identity/default",
  "created_at": "2026-05-21T10:23:45.000Z"
}
```

**Legacy format (file-fallback or pre-this-feature):**

```json
{
  "secret": [1, 2, 3, ..., 64],
  "pubkey_base58": "H8x4F2dCkP7zNcWqGmLpRtVxYbAhJoEsTuWnZQfXc4v"
}
```

Detection: parse as JSON, check `"secret" in keys`. Both shapes are valid; the absence of `secret` means stub. `keychain_ref` is informational (helps debugging); the actual keychain access uses the hardcoded service/account from Decision 1.

File mode 0600 on Unix; on Windows, owner-only ACL via existing helper.

### On-disk: `~/.mnemonic/README.txt`

Written once on first identity creation. Plain text, no JSON, ~5 lines:

```
This directory holds your Mnemonic identity and session state.

  identity.json — your Ed25519 public key (private key is in the OS keychain).
  token.json    — your current JWT for the Mnemonic webapp (regenerated on `mnemonic login`).

If you back up this folder, your private key is NOT included — back up via
`mnemonic identity push-to-webapp` or your OS keychain export.

Docs: https://mnemonik.xyz/docs/cli
```

### In-keychain entry

Service: `xyz.mnemonik.identity`. Account: `default`. Value (UTF-8 string):

```json
{"secret":[...64 bytes...],"pubkey_base58":"..."}
```

Same exact bytes as the legacy file format. No newlines, no extra whitespace. Cross-language byte-equality enforced by Wave 3 golden test.

### In-memory: `Identity` (Rust) / `Keypair` (TS)

Existing types in `core/src/identity/mod.rs` and `packages/sdk/src/identity/keypair.ts`. **No new fields.** What changes is the constructor: now sourced from `ensure()` instead of `load_or_create_keypair(path)`.

### In-memory: `Ticket` (server, ephemeral)

See Decision 12. Stored in `BootstrapTickets` LRU. Not persisted across server restarts (acceptable: tickets are 5-min TTL anyway).

### Server endpoints (mcp.mnemonik.xyz, `/api/cli-bootstrap/*`)

Implemented in `mcp/src/api.rs` (Wave 4 Task 12 + interop patch b13b0a0). All endpoints are unauthenticated — the capability is the ticket short_code itself; auth is enforced by ticket-bound origin + redeemer pubkey wrapping (see Decision 12).

| Method | Path | Body / params | Response | Caller |
|--------|------|----------------|----------|--------|
| POST | `/api/cli-bootstrap/issue` | `{wrapped_secret, eph_pub}` (wrap target = server x25519 pubkey from `/server-pub`) | `{ticket_id, short_code, expires_at}` | Webapp `IdentityPanel` "Send to CLI" button |
| POST | `/api/cli-bootstrap/issue-from-cli` | `{wrapped_secret, eph_pub, issuer_pubkey_base58}` | `{ticket_id, short_code, expires_at}` | CLI `mnemonic identity push-to-webapp` |
| POST | `/api/cli-bootstrap/redeem` | `{short_code, redeemer_eph_pub}` | `{wrapped_secret, eph_pub}` (re-wrapped to redeemer) | Both: CLI `pull-from-webapp` and Webapp `/install?pull=<short_code>` |
| GET | `/api/cli-bootstrap/redeem/{ticket_id}` | — | same shape as POST redeem | Legacy by-UUID variant; new clients use POST by short_code |
| GET | `/api/cli-bootstrap/server-pub` | — | `{server_pub_x25519_base64}` | CLI `push-to-webapp` (to wrap secret before issue) |

Single-use enforced atomically inside `BootstrapTickets::consume_by_short_code` / `consume(ticket_id)`. Second redemption attempt returns HTTP 410 + body `{"error":"ticket already redeemed (or expired)"}`. Server restart invalidates all in-flight tickets per Decision 18.

### CLI argv: `mnemonic identity <subcommand>`

```
mnemonic identity status [--json]
  → exit 0 (synced) | 3 (diverged) | 1 (no identity)
  → stdout: human or JSON status report

mnemonic identity pull-from-webapp <short-code-or-ticket-id>
  → exit 0 (redeemed) | 1 (bad code) | 2 (network) | 4 (expired/already-redeemed)
  → stdout: confirmation with new pubkey
  → side-effect: keystore.set, file rewrite if needed

mnemonic identity push-to-webapp [--qr-only|--code-only]
  → exit 0 (ticket issued) | 2 (network)
  → stdout: short_code + URL + QR (text-mode QR or PNG if TTY supports)
  → side-effect: ticket sits on server for 5 min
```

## Dependencies

### New packages

**`core/Cargo.toml`:**
- `keyring = "2"` — cross-platform OS keychain wrapper. Maps to macOS Keychain, Linux Secret Service (via D-Bus), Windows Credential Manager.
- `crypto_box = "0.9"` — x25519 sealed-box for ticket secret wrapping. Already a transitive dep via `solana-sdk`; promoted to direct for clarity.

**`packages/cli/package.json`:**
- `@napi-rs/keyring`: `^1.0.0` — Node binding over the same Rust `keyring` crate.
- `qrcode-terminal`: `^0.12.0` — print scannable QR to TTY for `push-to-webapp`.
- `@noble/curves`: `^1.4.0` — x25519 sealed-box operations on the Node side (already a transitive of `@noble/ed25519`, promoted to direct).

**`webapp/package.json`:** no new deps. Existing `qrcode.react` (or equivalent) is reused for in-page QR generation.

### Removed packages

`load_or_create_keypair(path)` is removed in the same PR that introduces `ensure()`. No deprecation cycle — pre-1.0, no documented external consumers of `mnemonic-core`. Callsites in `mcp/` are updated to `ensure()` directly.

### Using existing (from project)

- `bs58` (Rust + Node) — pubkey encoding, unchanged
- `ed25519-dalek` (Rust) / `@noble/ed25519` (Node) — keypair gen + signing, unchanged
- `serde` / `serde_json` (Rust) — identity.json (de)serialization
- `uuid` (Rust) — ticket IDs
- `tracing` (Rust) — identity creation announces via `tracing::info!`; see Decision 7 for the per-transport subscriber routing (stdio → stdout, HTTP → operator's choice). The mcp-server does NOT route identity-creation records to stderr — that would break Cursor/Claude Desktop JSON-RPC framing.

### Devdependencies

- `tempfile` (Rust dev-dep) — already present, used for `MemoryKeyStore` integration tests
- `vitest` (Node) — already present, used for `KeyStore` unit tests

## Testing Strategy

### Unit tests (every PR)

**`core/src/identity/ensure.rs` (Rust, `cargo test`):**
- `ensure_creates_when_absent` — empty dir → keypair generated, keychain entry set, stub file written
- `ensure_reads_existing_stub` — pre-populated stub + keychain entry → returns same pubkey
- `ensure_migrates_legacy_to_stub` — pre-populated legacy file → keychain entry set, file rewritten to stub, same pubkey preserved
- `ensure_falls_back_to_file_when_keychain_unavailable` — `MemoryKeyStore` rigged to return `PlatformUnavailable` → legacy file written, single stderr line emitted
- `ensure_recovers_on_keychain_set_then_file_write_fail` — file rewrite fails after keychain set → keychain entry removed, error propagated, legacy file intact

**`core/src/identity/keystore.rs` (Rust):**
- Round-trip on `MemoryKeyStore` (set → get → equal)
- `available()` returns false on simulated unavailable platform

**`packages/cli/src/identity/ensure.ts` (Node, vitest):**
- Same 5 scenarios as Rust above, mirrored in TS
- Plus: `ensure_throws_IdentityRequiresKeystore_on_stub_without_available_store` — typed error propagates correctly through SDK

**`packages/cli/src/commands/identity.ts` (Node, vitest):**
- `status` returns `synced` when JWT.sub equals local pubkey
- `status` returns `diverged` when JWT.sub differs
- `status` returns `webapp-unknown` when token.json absent
- `pull-from-webapp` rejects expired ticket
- `push-to-webapp` issues a ticket and prints both QR and short code

### Cross-language interop tests (new category, Wave 3)

Run from a Bash script in CI matrix. Steps:

1. Rust binary writes identity → Node CLI reads it → both report identical pubkey + base58.
2. Node CLI writes identity → Rust binary reads it → same.
3. Pre-populate legacy file → run Rust migration → Node reads result → equal. Then run Node migration from a fresh legacy file → Rust reads → equal.
4. Golden test: keychain entry value bytes equal across languages — both serialize to byte-identical `{"secret":[...],"pubkey_base58":"..."}` with the same key order.

### Integration tests (every PR)

- Node CLI E2E (vitest + spawned subprocess): `rm -rf ~/.mnemonic && mnemonic sign "x"` succeeds on a CI worker with `MemoryKeyStore` substituted via env var `MNEMONIC_TEST_KEYSTORE=memory`.
- Rust mcp-server E2E (`cargo test -p mnemonic-mcp --test integration_bootstrap`): start stdio server with fresh `~/.mnemonic`, send `mnemonic_whoami` JSON-RPC, expect pubkey in response and identity file on disk.
- Ticket flow: webapp issues ticket via `httpmock`-style harness → CLI redeems → pubkey matches issuer.

### Manual smoke tests (pre-release checklist in tasks/)

Cross-platform matrix gated by Wave 5 sign-off:

| Platform | Keychain | Bootstrap | Migration | Status drift | Push-to-webapp |
|----------|----------|-----------|-----------|--------------|----------------|
| macOS 14 | Keychain | ✓ | ✓ | ✓ | ✓ |
| Ubuntu 22 + gnome-keyring | Secret Service | ✓ | ✓ | ✓ | ✓ |
| Ubuntu 22 headless (no D-Bus) | n/a | file-fallback | n/a | ✓ | ✓ |
| Windows 11 | Credential Manager | ✓ | ✓ | ✓ | ✓ |
| Docker alpine | n/a | file-fallback | n/a | ✓ | ✓ |

### E2E tests (release pipeline, not PR-gating)

Webapp `/install` → click "Install in Cursor" → deeplink opens → Cursor MCP config applied → first MCP call from Cursor chat authenticates and returns expected pubkey. Verified by Playwright MCP against staging.

**Named drift pin-point coverage (user-spec AC §Layer 2 last bullet — 4 sub-criteria):**

| Drift case (from user-spec §Зачем) | Where verified |
|------------------------------------|-----------------|
| **Cursor 0.1.5 sign mismatch** (CLI keypair A, JWT minted under B, sign fails) | Wave 3 cross-language interop tests (T9 keychain.sh) + integration test that mints a JWT under the keychain pubkey and signs a memory; pubkeys equal end-to-end. |
| **IDE OAuth manual paste** ("pending bundle owner mismatch") | Playwright E2E `webapp/e2e/install.spec.ts` — click install → assert subsequent MCP call uses the same `JWT.sub` as webapp localStorage pubkey, no manual paste. |
| **Webapp test fixtures generated fresh keypair on `/install`** | `webapp/e2e/install.spec.ts` runs with pre-seeded `localStorage["mnemonic.identity"]` — assertion that the page does NOT overwrite it on load. Fixture-handling lives in `webapp/e2e/_helpers.ts`. |
| **In-memory rollback invalidated JWT** | Pinned by Decision 18 as accepted behavior. Test: restart `mnemonic-mcp` mid-pull → CLI receives `ticket expired (or server restarted)` (exit 3), retries with fresh ticket and succeeds. Drift detector (`mnemonic identity status`) correctly reports `diverged` if JWT.sub stopped matching local identity. |

### Security review (Wave 5, read-only)

- `grep -r "secret" packages/cli/src/ core/src/identity/` for any log/print of raw secret bytes.
- Ticket short_code entropy check (32-char alphabet, 8 chars = 40 bits, acceptable for 5-min TTL + single-use)
- File permission check (`stat` after creation, expect `0600`)
- Verify ticket short codes / URLs / IDs are passed via stdin or env var in CLI redemption, not argv (shell history leak)
- Verify JWT-baked deeplink doesn't surface JWT in any server-side log (URL-encoded in query string is the failure mode; we use POST + native deeplink only)

## Agent Verification Plan

### Verification approach

1. **Cross-language keychain interop** — Wave 3 has a dedicated CI job that boots a Linux+gnome-keyring container, runs both Rust and Node sides against the same keychain entry, asserts byte-equality of secrets. Failure here blocks Wave 4.
2. **Keychain unavailability scenarios** — Docker container without D-Bus, headless Linux without keyring helper, macOS Keychain after `security lock-keychain`. Each scenario asserts that `ensure()` either fall-backs to file or completes silently with an unlocked OS.
3. **JWT-baked deeplink against staging webapp** — Playwright MCP step in pre-release: login on `staging.mnemonik.xyz`, click "Install in Cursor", capture generated `mcp.json`, assert `headers.Authorization` is present and the JWT decodes to a valid sub matching the webapp pubkey.
4. **Ticket flow E2E** — Wave 4 ships an httpmock-style harness that simulates webapp issue + CLI redeem (and vice versa) without requiring a live webapp.

### Tools required

- **Bash MCP** — file ops, keychain state inspection (`security`, `secret-tool`, `cmdkey`), test harness orchestration.
- **Playwright MCP** — webapp install-page flow, drift-warning modal interaction.
- **None of:** real Solana RPC (not exercised here), Arweave (same), production webapp.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `@napi-rs/keyring` and Rust `keyring` crate write incompatible bytes to the same entry on one platform | Medium | High (cross-surface drift returns) | Wave 3 cross-language interop tests are PR-blocking. Adding a new platform requires re-running the matrix. |
| OS keychain prompt loop on macOS for every `mnemonic sign` invocation breaks UX | Medium | Medium | Decision 3: lazy access + macOS "Always Allow" on first prompt. Documented in `mnemonic identity --help`. Worst case, instruct users to `security set-keychain-settings` once. |
| Headless Linux CI silently degrades to file-fallback while developer thinks keychain works → drift surfaces only on user machine | High | Medium | `mnemonic identity status` reports `storage: file (keychain unavailable: <reason>)`. Decision 7 prints the fallback reason at first creation. CI explicitly tests both keychain and file paths separately. |
| JWT in install deeplink is committed to a public repo by a user | Medium | High (token leak) | Webapp UI banner above download. JWT TTL 1h limits damage. Server-side gitleaks rules (`gitleaks-config.toml`) extended with a JWT pattern for `mc.mnemonik.xyz`. |
| Ticket MITM via SSL termination or browser malware between issue and redeem | Low | High (key theft) | x25519 sealed-box wraps secret with ephemeral pubkey; server holds ciphertext + does a momentary re-wrap. Combined with TLS, requires either compromise of the server's ephemeral private key (process memory) or both endpoints — significantly raises the bar. |
| Migration races between simultaneous Node CLI and Rust mcp-server startup | Low | Medium (split-brain identity) | File rewrite via tempfile + rename is atomic. Keychain `set` is idempotent. Worst case: both sides write the same secret with same pubkey — no functional break. Cross-language test exercises this via concurrent invocation. |
| Test matrix on Windows misses Credential Manager quirks (e.g. credential roaming, GPO disabling persistence) | Medium | Medium | Manual smoke on Windows 11 Pro before each release. Document GPO override path for enterprise users (file-fallback). |
| `qrcode-terminal` Unicode rendering breaks on legacy Windows terminals | Low | Low (cosmetic) | `mnemonic identity push-to-webapp --code-only` skips QR, prints short code only. |
| `BootstrapTickets` LRU evicts an in-flight ticket under high concurrency | Low | Low | Eviction returns 410 Gone — user retries within 5 min. Real-world rate is <<1000 concurrent tickets per server; raise the cap if it becomes a problem. |
| `keypair-sync` archival loses links from external blog posts / docs that referenced it | Low | Low | `work/completed/keypair-sync/MOVED.md` is git-permanent; the user-spec.md inside is preserved verbatim. |

## User-Spec Deviations

All entries below were `[PENDING USER APPROVAL]` in the initial draft; they were resolved in chat on 2026-05-21 and are now `[ACCEPTED]`. See `decisions.md` for the audit log.

### Deviation 1: Lazy keychain access at bootstrap — `[ACCEPTED]`

**User-spec implies:** `identity.ensure()` makes identity available after bootstrap.
**Tech-spec does:** Only pubkey is loaded from disk at bootstrap. The secret is fetched from keychain on-demand at first signing operation. `Identity` returned by `ensure()` carries a `SecretAccessor` closure, not the raw bytes.
**Why:** Avoids OS keychain prompt on benign commands like `mnemonic whoami` / `--version`. Marketplace gate (no prompts on install) requires this. macOS will still prompt at *first* signing — users hit "Always Allow" once. README ships this nuance.

### Deviation 2: x25519 wrapping for ticket secrets — broker trust model with momentary server plaintext — `[ACCEPTED]`

**User-spec implies:** secrets are never plaintext on the network.
**Tech-spec does:** Wrapping is on the network, but the server unwraps to its own ephemeral key on `/redeem` and re-wraps to the redeemer's submitted ephemeral. There is a sub-millisecond window where the server process memory holds the plaintext secret.

**Trust model (pinned):**

The Mnemonic webapp/MCP server is a **trusted broker** for the ticket-redemption step. A compromised server can already mint forged JWTs and serve malicious responses to any client; it can therefore already break the user's identity guarantees independently of this code path. Adding a momentary plaintext window during ticket re-wrap **does not expand the existing trust surface**: an attacker who can dump server process memory at the exact ms of a `/redeem` call can also impersonate the user via JWT signing.

Mitigations within this trust model:
- **Plaintext window is sub-millisecond** — secret bytes are in scope only for the duration of `crypto_box::open()` → `crypto_box::seal()`. No logging, no persistence, no async boundary that could extend the lifetime.
- **Single-use tickets** — `redeemed=true` flag flipped before the re-wrap completes; second attempt returns 410 Gone.
- **5-minute TTL** — windows where a memory dump even matters are bounded.
- **Server's static x25519 keypair is in-process only**, generated at server start. Not persisted. Server restart invalidates all in-flight tickets (acceptable per TTL).

What this trust model does *not* cover:
- Compromised CLI or browser endpoint — out of scope; same as any client-side key compromise today.
- Malicious server operator — out of scope; same as today's JWT issuance.
- Memory-dump attack on the running server during the precise re-wrap moment — explicitly accepted within the trust model.

If a deployment ever needs zero server-side plaintext, the fallback is the manual-paste flow (issuer prints encrypted blob, user copies to redeemer device, redeemer decrypts with a pre-shared symmetric key derived from a short passphrase). Not in scope for this feature.

### Deviation 3: New `crypto_box` direct dep on Rust side + `@noble/curves` direct dep on Node — `[ACCEPTED]`

**User-spec implies:** dependencies are unchanged except where strictly necessary.
**Tech-spec adds:** Two new direct dependencies for x25519 sealed-box. Both already exist as transitive deps but are now promoted to direct so version-pinning is explicit.
**Why:** Implicit transitive deps for cryptographic primitives are a maintenance footgun (auto-upgrades have caused breakage in this stack before — `solana-sdk` major bumps in 2025 silently changed x25519 backends).

### Deviation 4: Remove `load_or_create_keypair(path)` immediately, no deprecation cycle — `[ACCEPTED]`

**Original draft:** keep old function as a deprecated thin shim for one release cycle.
**Tech-spec does:** Remove `load_or_create_keypair(path)` in the same PR that introduces `ensure()`. All callsites in `mcp/` are updated. Pre-1.0 project, no documented external Rust consumers of `mnemonic-core`. Deprecation windows are for ecosystems with stranger consumers; we don't have those.

### Deviation 5: Webapp install-page sorts by detected platform, does not hide other buttons — `[ACCEPTED]`

**User-spec describes:** install deeplinks for Cursor / VS Code / Claude Desktop.
**Tech-spec adds:** `webapp/src/pages/install.tsx` detects platform via `navigator.userAgentData` (with `navigator.platform` fallback) and sorts the three install buttons with the detected-platform one first. All three buttons are always visible. ~5 LOC, not 30. No button is ever hidden — UA detection failing safely leaves the original order intact.
**Why:** Softer than UA-sniffing-to-hide. Sorting ages better — "Cursor on Linux while browsing on iPad" is a real edge case.

## Tasks (wave decomposition)

Tasks live as `tasks/<n>.md` files; this section lists their headlines and wave membership. Within a wave, tasks may run in parallel **only if** they don't touch shared files. Shared files for this feature: `core/src/identity/mod.rs`, `core/src/lib.rs`, `mcp/src/main.rs`, `packages/cli/src/index.ts`, `packages/cli/package.json`, `core/Cargo.toml`.

### Wave 1 — Rust keystore + ensure() (sequential within wave; shared files)

1. **`tasks/1.md` — Add `keyring` and `crypto_box` to `core/Cargo.toml`, re-export from `lib.rs`.** Smallest possible diff that compiles. Sequential dependency for the rest of Wave 1.
2. **`tasks/2.md` — `KeyStore` trait + `OsKeyStore` + `FileKeyStore` + `MemoryKeyStore` in `core/src/identity/keystore*.rs`.** Three impls + trait. Plus unit tests on `MemoryKeyStore`.
3. **`tasks/3.md` — `identity::ensure()` in `core/src/identity/ensure.rs` covering: create / read-stub / migrate-legacy / file-fallback paths.** Five unit tests. Removes `load_or_create_keypair`.
4. **`tasks/4.md` — Wire `ensure()` into `mcp/src/main.rs`.** Replace existing `load_or_create_keypair` call. Add stdio integration test that boots server on fresh `~/.mnemonic/` and serves `mnemonic_whoami`.

### Wave 2 — Node keystore + ensure() (mostly parallel; only shared file is package.json)

5. **`tasks/5.md` — Add `@napi-rs/keyring`, `qrcode-terminal`, `@noble/curves` to `packages/cli/package.json`; verify N-API binaries on all CI matrix platforms.** Sequential before 6/7.
6. **`tasks/6.md` — TS `KeyStore` + `OsKeyStore` + `FileKeyStore` + `MemoryKeyStore` in `packages/cli/src/identity/keystore.ts`.** Mirrors Wave 1 Task 2. Parallel with 7.
7. **`tasks/7.md` — `identity.ensure()` in `packages/cli/src/identity/ensure.ts` + entrypoint wire-up in `packages/cli/src/index.ts`.** Parallel with 6 if it stubs the keystore for its own tests.
8. **`tasks/8.md` — `Keypair.fromJSON` in `packages/sdk/src/identity/keypair.ts` learns both shapes.** Independent file from Wave 1; can run in parallel with 6/7.

### Wave 3 — Cross-language interop tests (audit-style, mostly read-only; PR-blocking)

9. **`tasks/9.md` — `tests/cross-lang/keychain.sh` script + CI job matrix.** Boots Linux+gnome-keyring container, runs Rust write → Node read, Node write → Rust read, migration round-trip. Asserts byte-equality on identity.json and keychain entry. New CI job in `.github/workflows/ci.yml`.
10. **`tasks/10.md` — Golden JSON byte-equality test.** Rust serializes a fixed keypair to keychain-entry bytes; Node does the same; bytewise diff. Run in unit-test layer of both languages.

### Wave 4 — Sync surfaces (parallelizable across components)

11. **`tasks/11.md` — `mnemonic identity status` subcommand.** Reads keystore + token.json, compares JWT.sub. Exit code semantics per Data Models. Includes vitest unit tests. Independent of Wave 4.12/13/14.
12. **`tasks/12.md` — Server endpoint `/api/cli-bootstrap/issue-from-cli` + extend `/redeem` to handle both origins.** `mcp/src/api.rs`. Unit tests via existing httpmock harness. Independent file from 11/13/14.
13. **`tasks/13.md` — `mnemonic identity pull-from-webapp` + `push-to-webapp` subcommands.** `packages/cli/src/commands/identity.ts`. Depends on 12 (server endpoint must exist). Includes ticket-flow E2E test against mock server.
14. **`tasks/14.md` — Webapp `/install` page deeplink generator + IdentityPanel drift-warning modal.** `webapp/src/pages/install.tsx` + `webapp/src/components/IdentityPanel.tsx`. Independent of 11/12/13.

### Wave 5 — Polish, archival, audits (final)

15. **`tasks/15.md` — Manual cross-platform smoke matrix.** Run the table from Testing Strategy. Sign-off recorded in `decisions.md`.
16. **`tasks/16.md` — Security review of key handling.** Read-only. Findings appended to `decisions.md`.
17. **`tasks/17.md` — Code review across the diff.** Read-only. Reviewer = a fresh agent.
18. **`tasks/18.md` — Archive `work/keypair-sync/`.** `git mv work/keypair-sync work/completed/keypair-sync` + create `work/completed/keypair-sync/MOVED.md`. Update any cross-references in `work/cursor-vscode-e2e-tests/manual-verify.md` and `work/mnemonic-cli/backlog.md`.
19. **`tasks/19.md` — Pre-deploy QA gate.** Single sign-off task; merges blocked until green.

Each task file follows the project's frontmatter convention:

```yaml
---
status: todo | in-progress | done | blocked
depends_on: [<list of task IDs>]
wave: 1 | 2 | 3 | 4 | 5
skills: [rust | typescript | webapp | security | qa]
reviewers: [<list of reviewer roles>]
---
```

Task bodies follow the existing `work/mnemonic-core/tasks/<n>.md` template — short description, file list, acceptance criteria, verification command.

