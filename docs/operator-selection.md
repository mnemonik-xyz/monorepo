# Explicit operator selection

The source SDK exports `parseOperatorList` and `connectOperator`. These support
a locally configured list of operators, including O1 and O2 on different hosts.
They do not discover servers from the Internet or automatically fail over.
This is source functionality; publication and live staging acceptance remain separate.

## Configuration

Store a non-secret JSON document with this shape. Replace both example hosts and
public-key placeholders before use; placeholders are deliberately rejected.

```json
{
  "version": 1,
  "operators": [
    {"id": "o1", "baseUrl": "https://o1-staging.example.com", "publicKey": "REPLACE_WITH_O1_PUBLIC_KEY"},
    {"id": "o2", "baseUrl": "https://o2-staging.example.com", "publicKey": "REPLACE_WITH_O2_PUBLIC_KEY"}
  ]
}
```

Origins must use HTTPS, without paths or trailing slashes. Each operator needs
a distinct ID, origin and Ed25519 key. Obtain public-key pins through the
deployment/operator channel independently of the endpoint being tested. A key
advertised in an agent card alone is not an independent trust anchor.

## Connect and switch

```ts
import { parseOperatorList, connectOperator } from '@mnemonik-xyz/sdk';

const operators = parseOperatorList(JSON.parse(configurationText));
const o1 = await connectOperator(operators, 'o1', {
  signer: clientSigner,
  jwt: o1AccessToken,
});
const o2 = await connectOperator(operators, 'o2', {
  signer: clientSigner,
  jwt: o2AccessToken,
});
await o2.client.whoami();
// o1.client still addresses O1. No pending operation was moved to O2.
```

Each connection first calls `mnemonic_operator_proof` with a fresh random nonce.
This request carries no token, cookie or token refresh. The server signs a
message that it composes from a fixed domain tag, its public origin and the
nonce. The SDK verifies the signature against the pinned key and the configured
origin. Only then does it create the authenticated client. A wrong key, invalid
signature, replayed nonce or different origin fails before any credential leaves
the client. Requests to the selected MCP origin reject redirects. Credentials and
token refreshers must belong to that operator; the helper does not copy them
from another client.

The proof request times out after 15 seconds by default. Pass
`{ proofTimeoutMs, signal }` as the fourth argument to change the limit or
cancel it. A timeout fails the connection; the SDK does not try another operator.
Operators must run a server version with `mnemonic_operator_proof`. Older
servers fail this check.

`verifyOperatorIdentity(operator, options)` runs the same proof without creating
a client.

The proof establishes key possession at connection time. It does not establish
protocol compatibility, storage availability, payment readiness or independent
administration. Authenticated checkpoints, retained bytes and external-parent
verification remain necessary for recovery and continuation through O2.

## Payment and failure handling

An unavailable O1 never causes a request to O2. A lost write response may hide
a completed payment. Keep the original operator, exact request bytes and
operation identifier for reconciliation. Do not submit it to O2 merely because
O1 timed out. Selecting O2 permits new explicitly initiated work; it does not
resolve old operations or transfer balances, sessions or receipts.

Artifact discovery remains a separate storage-index operation. A future public
operator directory can supply candidates, but must not silently replace trusted
pins or become authority to retry paid writes.

Validation: `npm test -w @mnemonik-xyz/sdk -- --run test/operators.test.ts test/client.test.ts`
from the repository root. See the [pilot runbook](../work/protocol-product/pilot-runbook.md)
for live recovery acceptance.

## Read-only live identity check

Build the SDK, then run:

```sh
node scripts/check-operators.mjs --operators /secure/operators.json
```

The operator file uses the configuration above with independently obtained
deployment public-key pins. The probe needs no login session. It calls only
`mnemonic_operator_proof`, sends no credentials, rejects redirects and times out
each request after 15 seconds. It prints public pins and verification status,
never raw server errors. Any failed check produces a nonzero exit.
Run its offline checks with `node --test scripts/check-operators.test.mjs` after
building the SDK. Successful proof establishes possession of the pinned key,
not upload, recovery or payment acceptance.
