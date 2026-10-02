# Read-only live observation probe

Build the SDK before running `scripts/protocol-live-evidence.mjs`. The probe
uses public HTTPS indexes/gateways, sends no credentials, and performs no upload,
payment or deployment. No independently pinned live fixture is currently
provided by the repository; loopback demo locators are not live-provider pins.

```sh
node scripts/protocol-live-evidence.mjs \
  --author '<independently-pinned-author>' --context '<synthetic-context>' \
  --head 'a2a:<signed-binding-hash>' --locator 'ar://<storage-id>' \
  --expected-envelope-sha256 '<sha256-of-original-signed-bytes>' \
  --submitted-at '2026-10-02T12:00:00.000Z' \
  --submission-evidence 'evidence/synthetic-submission.json#upload' \
  --samples 6 --interval-ms 10000
```

Replace the example timestamp with a separately recorded actual submission
observation, never the artifact's signed creation timestamp. `submitted-at`
and `submission-evidence` are optional as a pair and require all four fixture
pins. Timestamps must be nonfuture UTC with milliseconds. References must name
a public record without credentials, query parameters or whitespace. Neither
the timestamp nor the evidence reference is independently authenticated by the
probe; the reference is recorded, not fetched. Keep credentials and private
content out of arguments and redirected reports.

The optional envelope SHA-256 pin compares fetched bytes with independently
retained original bytes before signature/identity verification. Without it,
the report records a fetched digest but does not compare an original-envelope
pin. Author, context and signed binding hash are always checked in fixture mode.

Each fixture index scan reports its start and finish; successful gateway
verification has a separate timestamp. `visibility.providers` reports first
verified presence and eventual visibility for each source. With submission
provenance it reports elapsed time to completion of both scan and gateway
verification as a **visibility latency upper bound**, conditional on comparable
clocks. It is not exact ingestion lag. Initial positive visibility provides no
lower bound. Unavailable sources and exhausted scan budgets are unknown, not
proof that the fixture was absent. A positive bounded scan can still establish
that the pinned locator was observed even if more pages remain.

The existing `positive_fixture_observed` result still requires verified fixture
visibility in at least one index in every sample. An initially missing fixture
that appears later therefore has eventual visibility but exits 2. Exit 0 does
not clear the release gate, which remains `not_established`. The probe does not
establish full history, recipient opening, independent-operator continuation,
retention or sustained availability.

Pure summarizer/validation tests run without network or a live fixture:

```sh
node --test scripts/protocol-live-observation.test.mjs
```
