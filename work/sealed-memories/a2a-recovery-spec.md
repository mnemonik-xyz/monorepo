# A2A external storage, parent validation and recovery

Status: proposed implementation contract, 2026-10-01. This document specifies
follow-up work; it does not assert that the refactor or recovery is shipped.

Related: [task 14](tasks/14.md), #242, #233,
[storage decisions](../arweave-as-source-of-truth/decisions.md),
[index migration](../chain-agnostic/decisions.md),
[storage backends](../pluggable-storage/decisions.md).

## 1. Scope and invariants

The client constructs, encrypts and signs the complete A2A artifact, including
its context, previous-artifact reference and completed sealed stream manifest.
The operator verifies those bytes and uploads them unchanged. An operator's
ANS-104 transport signature is not the author's artifact signature.

There are exactly two modes: agent-owned local storage and externally anchored
Arweave storage. Hosted local memory storage is not a third mode.

Hosted MCP SQL may contain utility configuration, OAuth/payment/quota records
and reconstructible routing receipts. It must not contain A2A payload bytes,
ciphertext, grant blobs, decrypted content, content keys or embeddings. Reader
identities in receipts are public routing metadata, not access credentials.
SQL is neither an artifact store nor an authoritative lineage index.

Deleting receipts must not invalidate existing artifacts, prevent their client
recovery or prevent valid chain continuation. Losing OAuth/configuration or
billing state is a separate operational incident: this spec does not promise
that a deploy with missing credentials can keep accepting paid requests.

Existing SEALED_V1/GRANT_V1 and A2A signed-binding formats remain unchanged.
The A2A binding hash and the enclosed sealed memory hash are distinct; use the
existing verified binding hash for `a2a:<hash>` and `prev_id`. Do not substitute
the outer envelope hash, upload ID or enclosed sealed memory hash.

## 2. External storage and local recall (task 18)

Anchored ingestion validates client signatures and tool/JWT bindings before
upload. Invalid input and explicit hosted local mode must fail before upload or
charging. Upload the original signed bytes with deterministic discovery tags:
`App-Name=mnemonic-protocol`, `Mnemonic-Type=a2a`, `Producer=<author base58>`,
`Context-Id=<signed context>`, `Content-Hash=<verified A2A binding hash>`.
Producer identifies the signed artifact author, not the paying upload operator.

Return `attestation_id`, binding hash, `arweave_tx`, `locator=ar://<id>`, sealed
flag and anchored mode only after a bounded fetch verifies delivered bytes.
Delivery failure is not success or permission to fall back to hosted storage.
Pending receipts must not appear as delivered artifacts.

Replay uses content identity and externally verified delivery. Receipts are an
optimization; deleting them must not permit an artifact to be replaced or make
an existing artifact fail verification. Deterministic upload IDs depend on
operator signing key and upload format; do not promise identical IDs across
operator rotation or format-2/ANS-104 changes. Multiple locators can identify
the same binding. Financial nonce protection remains in payment infrastructure;
this spec does not claim exactly-once charging after its state is lost.

MCP recall may return receipt metadata as a convenience directory. It cannot
perform semantic recall over hosted sealed content. The SDK fetches original
bytes, verifies them, stores them on the agent's device and opens them locally.
Known-locator import requires an independently supplied expected author, never
an author trusted solely because the fetched envelope says so. Gateway calls
must carry no MCP JWT or signing secret.

Local mode performs signing, index writes, recall and open without hosted MCP,
payment or a gateway. Durable local storage is an explicit SDK store/CLI file;
a default in-memory store is session-only. Local storage loss cannot recover
unanchored artifacts from Arweave.

## 3. SQL-independent parent validation (task 19)

Proposed API: SDK attest options and MCP schema accept `prev_locator?: string`.
`prev_id` stays in the existing client-signed binding; the locator is a fetch
hint and does not grant authority. Roots omit both. A non-root requires a parent
locator; an older caller may use a receipt-resolved locator, but absence of the
receipt must produce `ParentLocatorRequired`, not `ParentNotFound`.

Fetch only a validated `ar://` identifier through a configured gateway, never
an arbitrary caller URL. Verify the parent's complete signed envelope and its
sealed/grant bindings using the shared verifier. Its verified A2A binding hash
must equal the child's signed `prev_id`, and contexts must match. Reject
self-links. No SQL row can bypass these checks, including ready receipts.

Preserve the current link eligibility rule: the child signer is the verified
parent author or a named recipient in a valid author-signed parent grant.
This permits a signed reference; it does not grant mutation of the parent,
exclusive ownership of a context, or a general write capability. Read grants
must not silently acquire broader write authority. A future context admission
policy is separate work; if stricter continuation authority is required, revise
this rule explicitly before implementation rather than inventing unsigned ACLs.

Ingestion verifies the immediate parent only. It does not claim complete
ancestry validation; client restoration performs graph validation (§5).
Sealed parents are checked without decrypting. The server receives no K.

Invalid parent bytes/hash/context/link eligibility fail before child upload.
Unavailable delivery gives a retriable `ParentUnavailable`; it must not be
reported as a cryptographic failure or silently accepted. Proposed baseline:
1 MiB per blob, one parent fetch per request, 10-second fetch timeout, no
redirect outside the configured gateway origin. Share these limits with SDK
fetching; expose bounded configuration, not unbounded caller overrides.

Local mode verifies the parent from the client's original signed bytes and
uses the same hash/context/link rules. Receipt loss must not affect acceptance.

## 4. External discovery (task 20)

Proposed SDK surface: `restoreA2AContext(contextId, {expectedAuthors, ...})`,
plus existing known-locator import. Expected authors are caller-pinned identities
or identities independently authenticated through verified AgentCards. Multiple
authors are explicit: a context may contain a recipient-authored continuation.
Knowing a context ID is not proof that every signer in it is trusted.

Configuration separates payload gateway URL from index endpoint and explicit
index endpoint. The index is standard Arweave GraphQL (for example
`https://arweave.net/graphql`) with block timestamps.
The owner filter is the Arweave address: base64url(sha256(public key)). Never equate upload operator address with author.
Query the app/type/context/Producer tags, paginate with opaque cursors and
validate every candidate from its fetched signed bytes. Tags, timestamps,
returned owner and ordering are discovery hints only.

The client recomputes IDs and rejects signature, author, context or signed
binding mismatches. Treat conflicting tags as rejected candidates with a
reported reason; a bad candidate must not replace a verified local entry.
Deduplicate verified content identity across locators. Preserve all valid forks;
order presentation deterministically by signed timestamp then artifact ID,
without treating timestamps as trusted consensus order.

Proposed defaults: 100 candidates/page, at most 100 pages and 10,000 candidates,
1 MiB/blob, four concurrent fetches, 10-second per-request timeout. Detect cursor
loops, cancellation, malformed responses and GraphQL errors. Never label a
budget-limited or interrupted scan complete; return a resumable checkpoint and
structured diagnostics. Validate checkpoints against context/authors/endpoint.

No MCP login, SQL or receipt endpoint is required for discovery/import/open.
A receipt directory can accelerate lookup but cannot be the only source.
External index lag is expected: a just-delivered known locator may work before
search does. Retry explicitly; an empty page does not prove no artifacts exist.

Current Arweave GraphQL indexing is a remaining provider dependency. This spec replaces
MCP SQL dependency, not all external availability dependencies. Legacy memo
readers may remain where already applicable; no new Solana memo writer is added.
Live endpoint coverage must be measured before claiming production enumeration.

## 5. Chain restoration and client-side open (task 21)

Restoration stages verified original bytes on the client's device. Build a graph
whose edges follow verified signed `prev_id` references. Fetch supplied known
parent locators when discovery leaves gaps; report unresolved parents otherwise.
Validate context and link eligibility on every edge, detect cycles, retain forks,
and reject conflicting/invalid entries. Cross-author parents require the caller's
trusted-author set; an untrusted parent is unresolved, not implicitly trusted.

Return separate states: `scan_exhausted`, `budget_exhausted`, `missing_parents`,
`invalid_candidates`, and `complete_to_heads`. Completeness is relative to
explicit caller-pinned head IDs whose ancestry reaches valid roots. Without
pinned heads, report verified discovered graph with `completeness=unknown`.
Neither a gateway nor an unsigned index can prove that no newer head or hidden
fork exists. Preserve previously verified local entries on partial restoration.

For each restored sealed artifact, client open verifies the author signature,
recipient grant or author wrap, sealed memory binding and complete chunk chain
before releasing plaintext. Check indices, finality, previous chunk hashes,
manifest/head and AEAD authentication. Never present a partial decrypted stream
as a successfully restored artifact. Non-recipients can fetch public ciphertext
but cannot decrypt; discovery is not authorization.

The real SDK/WASM HTTP integration test must:

1. Write a root, child and grandchild, including a completed sealed chunk-chain
   artifact, through MCP to a separate external-storage test service.
2. Save signed expected heads and independently trusted author keys on the client.
3. Delete every MCP A2A receipt/reader row and discard client index and sessions.
4. Disable MCP for restore; a fresh client enumerates the external index, fetches,
   verifies and reconstructs ancestry, opens as author and named recipient.
5. Assert outsider failure and exact original plaintext/bytes/hash preservation.
6. Restart MCP with required operational configuration and empty receipts; append
   a valid child using an external parent locator. Prove no SQL ancestry lookup
   is required for correctness.

Add adversarial tests for missing pages/parents, wrong hashes/context/signatures,
forged tags/grants, cycles, forks, duplicate locators, provider outage, index lag,
size/time/cursor limits and repaired-public-hash ciphertext tampering. Mocked
index tests prove code behavior, not production index availability. Record a
separate live discovery smoke result without leaking private client keys.

## 6. Minimal implementation map

| Files | Change | Task |
|---|---|---|
| `mcp/src/tools.rs`, `mcp/src/mcp.rs` | unchanged-byte external upload, explicit mode, metadata recall; parent locator schema/dispatch and external verification | 18, 19 |
| `core/src/storage/sqlite.rs` | receipt metadata only; remove authoritative parent existence checks and memory writes from this A2A path | 18, 19 |
| `core/src/arweave/mod.rs` | bounded delivery and explicit upload-format support; do not assume local format-2 equals ANS-104 | 18 |
| `mnemonic-a2a/src/signed.rs`, `core/src/codec/a2a/signed.rs` | pure verified bindings and shared edge checks; no SQL dependency | 19, 21 |
| `packages/sdk/src/a2a.ts`, `client.ts`, `types.ts`, `index.ts` | local durable store, locator import, parent hint, discovery, diagnostics and graph reconstruction | 18–21 |
| `core/src/arweave/graphql.rs` | reuse schema behaviour; extend only if shared native discovery needs it | 20 |
| `packages/cli/src/a2a-index.ts`, `commands/a2a.ts`, `bin/mnemonic.ts` | agent-local index, mode/restore/parent options; identity-scoped durable writes | 18–21 |
| `mcp/tests/integration_a2a_mcp_tools.rs`, SDK A2A tests and `scripts/test-sealed-a2a-e2e.mjs` | separate storage service, schema/dispatch, SQL-loss restore and continuation proofs | 18–21 |
| `docs/sealed-a2a.md`, `docs/tools.md`, SDK/CLI docs | actual storage boundary, recovery guarantees and limits | 18–21 |

## 7. Closure and pending decisions

Tasks 18–21 close only when their tests and documented contracts pass. Do not
mark completed from a proposed spec or from earlier tests of SQL artifact storage.

Task 14 still requires native/SDK conformance, sealed MCP input/output,
client-only open, verified recipient cards, threat notes and integration of
chunks into the live #61 SSE chain. A completed stored chunk manifest alone does
not satisfy its stream item. For this branch's SQL-independent recovery promise,
tasks 18–21 are additional explicit dependencies. This does not retroactively
claim that external discovery was in #242's original acceptance criteria.

#242 closes after its full recorded scope is implemented and tested, including
any still-required #74 DID discovery and #61 stream integration; an owner-approved
scope change must be recorded explicitly. #233 is an umbrella, not automatically
closed by these tasks. Task 14 and #242 remain open during spec/implementation.

Proposed API names, limits and link eligibility above require review. The fixed
owner requirements (client signs everything; hosted SQL stores no memories;
SQL loss must not destroy anchors) are not reopened by this proposal. Discovery
completeness beyond pinned heads and admission rights beyond signed linkage
are deliberately not promised.
