# Pilot preparation and recovery runbook

Status: source-candidate preparation, not pilot approval. Baseline: PR #268,
merge `26a9550`. No package publication, deployment, paid transaction or live
rollback was performed while preparing this document.

Task 12 depends on tasks 5, 6 and 8. This preparation does not clear those gates
or A-17. Use the [source capability matrix](../../docs/source-capabilities.md),
[release evidence](release-evidence.md) and [shipping plan](shipping-plan.md)
together; a successful build is not evidence of a running release.

## Deployment ownership

The hosted MCP deployment workflow is
[deploy-mcp.yml](../../.github/workflows/deploy-mcp.yml). It joins the tailnet,
uses the coding-fabric-rendered stack (default `/opt/mnemonik-server`), preserves
its existing Compose project and `/data` and `/keypair` mounts, and reloads shared
Caddy. Do not substitute this repository's standalone nginx/Certbot Compose
stack on that machine. The [standalone guide](../../mcp/deploy/README.md) applies
to separately provisioned hosts.

Image builds and publication are separate from deployment. The image workflow
builds `mcp/Dockerfile` for Linux amd64. Deployment is manual; `main` and arbitrary
non-version refs map to mutable `latest`, while `v*` and `sha-*` refs select image
tags. A tag is not an immutable digest. Before an approved change, record the
resolved digest and verify that the selected tag still resolves to it. The
current workflow does not accept digest-pinned deployment as a declared feature.

The webapp uses Cloudflare Pages Git integration; the GitHub webapp workflow
checks its build. Neither a generic Pages check nor MCP health establishes all
webapp capabilities. In particular the pinned npm WASM package lacks sealed
opening; the page reports unsupported decryption. Releasing a suitable artifact
and testing the installed page remains separate work.

## Observed image build

On 2026-10-02, [image run 37006876302](https://github.com/mnemonik-xyz/monorepo/actions/runs/37006876302)
completed successfully for source `26a9550a89465f3bf00de2267d752037c82a625d`.
The build log records publication of `ghcr.io/mnemonik-xyz/mnemonic-mcp:sha-26a9550`
and `:latest` with manifest-list digest
`sha256:113a30acc0905ea1b1af36b06521540981d0b196eae078dde102a8ead6099348`.
This establishes CI image build/publication, not deployment, startup, migration
compatibility, released SDK installation or A-17 acceptance. Resolve the selected
tag again before any approved deployment; tags can move.

## Candidate evidence record

Create one record per candidate; leave unknown fields explicitly `pending`.
Do not infer released versions from source `package.json` values. Store only
public identifiers and evidence references here, never keys, environment dumps,
payment proofs or decrypted content.

| Field | Evidence required |
|---|---|
| Source | Full commit, clean tracked-source status, protocol/reader compatibility |
| SDK and CLI | Actual registry version or local candidate tarball label, tarball SHA-256, npm integrity, build command and toolchain |
| WASM | Included file digest and exports; source/build relationship |
| MCP | Build run URL and conclusion, image digest, target architecture |
| Webapp | Build commit, Pages deployment identifier, observed public capability checks |
| Storage and discovery | Configured adapter names, trusted endpoints, pinned synthetic fixture, observed submission timestamp |
| Operators | Independently pinned public keys, separate deployment ownership and configured storage origins |
| Rollback | Previous image digest/tag, reader/schema compatibility results, backup restore evidence |
| Acceptance | Install/save/restore/switch/continue evidence from these exact candidate artifacts; separately dated live results |

Local tarballs may be inspected and installed in disposable directories before
publication. `npm pack` does not run this repository's `prepublishOnly` build:
build the SDK/WASM and CLI first, then verify the tarball includes the compiled
JavaScript, declarations and WASM. Install outside the workspace so symlinks,
workspace source imports and stale build output cannot satisfy the drill.
Do not call this a released-artifact A-17 pass until actual released artifacts
are installed and exercised.

The [candidate package probe](../../scripts/check-local-packages.mjs) packs
already-built SDK/CLI output, installs both tarballs outside the workspace, then
runs standalone identity creation, sealed local save and fresh-process open:

```sh
node scripts/check-local-packages.mjs
```

Evidence is written to `target/protocol-package-drill/report.json` beside the
tarballs, with SHA-256 and npm integrity values. Check the command exit status,
report timestamp and tarball hashes together. The report's `checkout_revision`
is the checkout observed by the probe, not proof that the prebuilt JavaScript
or WASM came from that revision. Rebuild and record toolchain provenance before
using the outputs as a release candidate.

Dependency installation can contact npm; the subsequent CLI processes replace
`globalThis.fetch` with a rejecting function. This is a check of these local
paths, not an operating-system network sandbox. The probe uses a disposable
identity directory and deletes it afterward. It does not exercise hosted
storage, checkpoints, operator/backend migration, payment, rollback or a
published registry release; those remain separate acceptance steps.

## Synthetic staging and acceptance

Provision staging separately from production only through an authorized change.
Use synthetic content, separate operator identities, separate databases and
storage namespaces. Name supported adapter configurations explicitly; this
runbook does not assume a provider test network is supported.

1. Install pinned candidate packages into a fresh runtime. Save the exact signed
   bytes and operation ID before delivery, including public-write consent where
   applicable. Record author, context, signed heads and locators independently.
2. Record storage submission time from the actual delivery observation. Measure
   time to exact-byte refetch and discovery separately. A signed creation time
   is not an ingestion timestamp.
3. Sign and export a checkpoint. Disable the first operator, remove only its
   disposable staging receipts, and recover in a fresh recipient runtime.
4. Copy exact signed bytes through the supported storage migration path. Disable
   the source backend and recover using the authenticated destination manifest.
   Verify parent/author/head completeness before appending through operator two.
5. Exercise missing bytes, changed bytes, wrong recipient, stale checkpoint,
   unavailable discovery and exhausted scan budgets. Partial recovery must stay
   explicitly partial; no empty-index success may substitute for pinned heads.
6. Exercise payment retries and rollback below. Record real-provider results
   separately from local mocked services and stop on a failed required gate.

The existing local drill command and its scope are in [release evidence](release-evidence.md).
It is useful regression evidence, but does not replace the installed-package,
live-provider or separately deployed operator runs.

## Backup responsibilities

Client recovery requires more than a receipt database backup. Preserve signing
and decryption identities, independent trusted owner/author pins, a current
owner-signed checkpoint, exact signed envelopes needed for unresolved retries,
and the authenticated locator manifest after migration. Keep the encrypted
backup independently of the operator and retain its passphrase separately.

The SDK `createRecoveryBackup` / `openRecoveryBackup` APIs authenticate the
checkpoint and owner identity. Restore with an independently pinned owner and
expected artifact kind/scope; do not trust an owner key merely because it was
inside a downloaded backup. Test decryption on a fresh runtime. Restoring only
public verification metadata cannot reconstruct missing recipient secrets.
Checkpoints establish the declared heads, not undisclosed newest history.

The operator must preserve the entire consistent SQLite database and its stable
identity/configuration, including `paid_operations`, `delivery_operations`,
legacy paid-artifact metadata, receipts, quotas and authentication state. Use
SQLite's online backup API, or stop writers and take a consistent database/WAL
snapshot; copying a live `.db` alone can omit committed WAL state. Encrypt
backups and test restoration on an isolated host before relying on them.

Do not publish backup contents or print secrets during diagnosis. Record backup
identifier, timestamp, checksum, restore-test result and custodian separately.
Historical paid rows may retain source bytes until verified resubmission/drain;
backup retention includes that exception. Never purge them as an automatic
migration or treat deletion as a substitute for financial reconciliation.

## Financial incidents and rollback

The shared ingestion path supports Universal Paywall exact settlement before
delivery; other paid rails fail closed. See [payment and retry behavior](../../docs/artifact-payment-retries.md).
Keep the original signed bytes and same operation ID for retries. Do not issue
a new operation to work around a delivery failure after settlement.

On database loss, external byte verification may prove delivery while payment
remains unknown. Stop affected paid writes and reconcile durable/provider
receipts before accepting another payment. On settled delivery failure, retry
within the existing operation; terminal failure exposes `remedy_pending`.
An operator records a remedy evidence reference only after the actual remedy
is established. No automatic refund or live refund support is claimed.

Before rollback, freeze new writes, preserve a fresh consistent financial
snapshot and inventory the live mounts and public server identity. Verify the
previous binary can read the current schemas, sealed bytes and payment receipt
states in staging. If it cannot, keep writes disabled and roll forward with a
compatible reader instead. Never restore an older database over newer paid
operations to make an older binary start: that can lose settlement/replay state.

Run an approved rollback through the existing fabric workflow with a verified
previous image tag/digest. Preserve all live volumes and original signed
artifacts. The workflow's health checks and image replacement do not themselves
prove financial or reader compatibility. Afterward verify the same identity,
known artifact bytes, status of settled operations, same-operation no-second-
charge retries, recipient restore and continuation. Record results before
reopening writes. Failure leaves the candidate unapproved and writes stopped;
preserve evidence for reconciliation rather than deleting records.

## Operational observations and release decision

For each staging run record delivery/refetch latency, index lag from observed
submission, request/failure counts with denominators, partial restore reasons,
unresolved payment count and remedy age. Record the observation interval and
provider configuration. These are required measurements, not assertions that
production metrics exporters or an SLA already exist.

Pilot approval requires the candidate record, restore and rollback evidence,
capability limitations, task 5 positive live evidence, task 6 accurate docs and
task 8 demonstration. An owner must assign incident/remedy responsibility and
approve any publication/deployment. Counsel-dependent retention, permanent
storage, deletion, service liability and refund terms remain owner/counsel work.
Engineering evidence does not establish legal compliance or customer demand.
