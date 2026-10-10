# Surface and responsibility map

Status: planned boundaries. See [gaps.md](gaps.md) for current exceptions.

## Layers

| Layer | Runs where | Responsibility | Existing module | Paid value |
|---|---|---|---|---|
| Agent application | Customer runtime | Select memory and recovery policy | Customer integration | Useful integration |
| Client | Agent device | Build, encrypt, sign, open and rank | `packages/sdk`, CLI, extension, webapp | Integration support |
| Portable protocol | Client and operator | Encode, hash and verify | `core/src/codec`, `core/src/sealed`, WASM | Open interoperability |
| Operator API | Hosted or agent-local process | Auth, validate and coordinate delivery | `mcp/src/mcp.rs`, `api.rs`, `tools.rs`, `sealed_routes.rs` | Managed execution |
| Payment | Hosted operator | Quote, authorization, settlement and remedy | `mcp/src/payment.rs`, `pricing.rs`, paid-operation modules | Predictable billing |
| Storage adapter | Operator and client | Upload, fetch and enumerate | `core/src/arweave`, future storage adapters | Supported backend access |
| External infrastructure | External provider | Store bytes and expose lookup | Arweave, ArDrive Turbo, configured gateways | Provider service |
| Recovery | Agent device | Verify graph and rebuild index | `core/src/restore`, `rebuild`, SDK A2A | Migration and continuity |
| Operations | Operator | Measure availability and handle incidents | Planned reporting | Monitoring and support |

The existing operator is a service using external infrastructure.
An independent Mnemonik network requires multiple interoperable operators and meaningful client choice.
It does not require a native token.

## Data locations

| Data | Agent device | Hosted operator database | External storage |
|---|---|---|---|
| Signing and decryption keys | Yes; secure backup required | Never | Never |
| Plaintext private memory | Yes | Never in the target private path | Never |
| Signed sealed artifact and grants | Yes | No durable copy | Yes for anchored mode |
| Explicit public artifact | Yes | No durable copy | Yes for anchored mode |
| Embeddings and recall index | Yes | No memory index in target path | Only inside allowed signed artifact fields |
| Locator, hash and operation receipt | Yes | Reconstructible metadata | Discovery tags may expose metadata |
| OAuth, quotas and financial replay state | Optional client session | Yes | Payment rail records where applicable |
| Recovery checkpoint | Independent client backup | Optional convenience copy | Optional; never the only copy |

OAuth means Open Authorization. A receipt is an operator report, not the artifact's source of truth.
Database loss must not destroy delivered memory. Financial state loss remains a separate incident.

## Trust boundaries

- The author signature binds the artifact. The upload operator signature binds transport only.
- An authenticated payer does not become the artifact author or an authorized reader.
- A locator selects bytes. It does not establish a trusted author.
- Tags and index results are hints. Verify fetched signed bytes before import.
- A gateway response is not evidence that no other head exists.
- Private recall runs locally. A hosted recall key would grant the server decryption capability.
- A read grant cannot erase copies already obtained by a reader.

External storage exposes ciphertext and routing metadata publicly where applicable.
Document author, context and timing exposure before customer approval.
Encrypting content does not hide every relationship between artifacts.

## Module rules

Keep portable verification in `core/`. Keep payment policy in `mcp/`.
`core/` must not depend on `mcp/`.
Share verification between SDK and operator through existing core and WebAssembly bindings.
Use one delivery coordinator behind each hosted write entry point.
Use pure parent checks without SQL authority.
Do not hold a SQLite mutex during network requests.
Do not add a new Solana memo writer as a recovery prerequisite.
