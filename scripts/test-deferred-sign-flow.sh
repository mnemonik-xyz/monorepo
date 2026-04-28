#!/usr/bin/env bash
#
# scripts/test-deferred-sign-flow.sh — smoke harness for Task 5
# (browser-mediated signing flow, Decision 12).
#
# Walks through the full deferred-signing round-trip without a browser:
#   1. Mint a test JWT bound to a fresh Ed25519 keypair (sub = base58 pubkey).
#   2. Call `tools/call mnemonic_sign_memory` over /mcp — the server defers
#      and returns `{status: "awaiting_signature", correlation_id, ...}`.
#   3. GET /api/pending/<correlation_id> — fetch the canonical-CBOR bytes.
#   4. Sign the bytes locally with `cargo run --example sign_pending`
#      using the same keypair that backs the JWT (so signer_pubkey == jwt.sub).
#   5. POST /api/sign-callback with the COSE_Sign1 base64.
#   6. Call `tools/call mnemonic_recall` and assert the just-persisted
#      attestation is returned.
#
# Prerequisites:
#   - server running with `STORAGE_MODE=local PAYMENT_MODE=none` on
#     `http://127.0.0.1:3000` (or override via $MCP_BASE_URL),
#   - `MCP_JWT_SECRET` set to the same secret the server is using,
#   - `jq`, `openssl`, and `base64` on PATH,
#   - `cargo` on PATH (for the keypair-signing helper + JWT minter).
#
# Exit codes: 0 on success, non-zero on any failure.

set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

MCP_BASE_URL="${MCP_BASE_URL:-http://127.0.0.1:3000}"

if ! command -v jq >/dev/null; then
    echo "FAIL: jq is required (brew install jq)" >&2
    exit 10
fi

if [[ -z "${MCP_JWT_SECRET:-}" ]]; then
    echo "FAIL: MCP_JWT_SECRET env var is required (32-byte base64)" >&2
    exit 11
fi
export MCP_JWT_SECRET

# 1. Generate a fresh keypair via the example helper. The helper prints
#    `signer_pubkey=...` and `secret_base64=...` on stderr; capture both.
echo ">>> [1/6] Generating test keypair (sign_pending --no-secret)" >&2
cargo build -p mnemonic-mcp --example sign_pending --quiet
TMP_KP_OUT="$(mktemp)"
trap 'rm -f "$TMP_KP_OUT" "${TMP_CBOR:-}" "${TMP_COSE:-}"' EXIT
echo "" > /tmp/_empty_cbor.bin
target/debug/examples/sign_pending --cbor-file /tmp/_empty_cbor.bin >/dev/null 2>"$TMP_KP_OUT" || true
SIGNER_PUBKEY="$(grep -E '^signer_pubkey=' "$TMP_KP_OUT" | head -n1 | sed 's/^signer_pubkey=//')"
SECRET_B64="$(grep -E 'secret_base64=' "$TMP_KP_OUT" | head -n1 | sed 's/.*secret_base64=//')"
if [[ -z "$SIGNER_PUBKEY" || -z "$SECRET_B64" ]]; then
    echo "FAIL: keypair extraction failed; helper stderr:" >&2
    cat "$TMP_KP_OUT" >&2
    exit 12
fi
echo "    signer_pubkey=$SIGNER_PUBKEY" >&2

# 2. Mint a JWT for that subject.
echo ">>> [2/6] Minting test JWT for sub=$SIGNER_PUBKEY" >&2
cargo build -p mnemonic-mcp --bin mint-test-jwt --quiet
TOKEN="$(target/debug/mint-test-jwt --sub "$SIGNER_PUBKEY")"
if [[ -z "$TOKEN" ]]; then
    echo "FAIL: mint-test-jwt produced empty token" >&2
    exit 13
fi

# 3. Call mnemonic_sign_memory over /mcp — expect awaiting_signature.
echo ">>> [3/6] Calling mnemonic_sign_memory over /mcp" >&2
SIGN_BODY=$(cat <<EOF
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemonic_sign_memory","arguments":{"content":"deferred sign flow smoke test memory","tags":["smoke"]}}}
EOF
)
SIGN_RESP="$(curl -sS -X POST "$MCP_BASE_URL/mcp" \
    -H "content-type: application/json" \
    -H "authorization: Bearer $TOKEN" \
    -d "$SIGN_BODY")"
# Streamable-HTTP NDJSON: take the first line.
SIGN_FRAME="$(printf '%s' "$SIGN_RESP" | head -n1)"
INNER="$(printf '%s' "$SIGN_FRAME" | jq -r '.result.content[0].text')"
STATUS="$(printf '%s' "$INNER" | jq -r '.status')"
CORRELATION_ID="$(printf '%s' "$INNER" | jq -r '.correlation_id')"
APPROVE_URL="$(printf '%s' "$INNER" | jq -r '.approve_url')"
if [[ "$STATUS" != "awaiting_signature" ]]; then
    echo "FAIL: expected status=awaiting_signature, got: $INNER" >&2
    exit 14
fi
echo "    correlation_id=$CORRELATION_ID  approve_url=$APPROVE_URL" >&2

# 4. GET /api/pending/<id> — fetch canonical-CBOR.
echo ">>> [4/6] GET /api/pending/$CORRELATION_ID" >&2
TMP_CBOR="$(mktemp)"
HTTP_CODE="$(curl -sS -o "$TMP_CBOR" -w '%{http_code}' \
    -H "authorization: Bearer $TOKEN" \
    "$MCP_BASE_URL/api/pending/$CORRELATION_ID")"
if [[ "$HTTP_CODE" != "200" ]]; then
    echo "FAIL: /api/pending returned HTTP $HTTP_CODE" >&2
    cat "$TMP_CBOR" >&2
    exit 15
fi
CBOR_SIZE="$(wc -c <"$TMP_CBOR")"
if (( CBOR_SIZE < 10 )); then
    echo "FAIL: canonical-CBOR body too small ($CBOR_SIZE bytes)" >&2
    exit 16
fi
echo "    fetched $CBOR_SIZE bytes of canonical-CBOR" >&2

# 5. Sign the CBOR with the same keypair → COSE_Sign1 base64 on stdout.
echo ">>> [5/6] Signing canonical-CBOR locally" >&2
COSE_B64="$(target/debug/examples/sign_pending \
    --cbor-file "$TMP_CBOR" \
    --secret-base64 "$SECRET_B64")"
if [[ -z "$COSE_B64" ]]; then
    echo "FAIL: sign_pending produced empty COSE base64" >&2
    exit 17
fi

# 6a. POST /api/sign-callback.
echo ">>> [6/6] POST /api/sign-callback" >&2
CB_BODY="$(jq -n \
    --arg cid "$CORRELATION_ID" \
    --arg cose "$COSE_B64" \
    --arg pk "$SIGNER_PUBKEY" \
    '{correlation_id: $cid, cose_signed_bytes: $cose, signer_pubkey: $pk}')"
CB_RESP="$(curl -sS -X POST "$MCP_BASE_URL/api/sign-callback" \
    -H "content-type: application/json" \
    -H "authorization: Bearer $TOKEN" \
    -d "$CB_BODY")"
CB_STATUS="$(printf '%s' "$CB_RESP" | jq -r '.status' 2>/dev/null || echo error)"
if [[ "$CB_STATUS" != "ok" ]]; then
    echo "FAIL: /api/sign-callback did not return status=ok: $CB_RESP" >&2
    exit 18
fi
ATT_ID="$(printf '%s' "$CB_RESP" | jq -r '.attestation_id')"
echo "    persisted attestation_id=$ATT_ID" >&2

# 6b. Recall: search for the just-stored memory.
echo ">>> [recall] Verifying mnemonic_recall returns the persisted row" >&2
RECALL_BODY=$(cat <<EOF
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mnemonic_recall","arguments":{"query":"deferred sign flow smoke test","limit":5}}}
EOF
)
RECALL_RESP="$(curl -sS -X POST "$MCP_BASE_URL/mcp" \
    -H "content-type: application/json" \
    -H "authorization: Bearer $TOKEN" \
    -d "$RECALL_BODY")"
RECALL_FRAME="$(printf '%s' "$RECALL_RESP" | head -n1)"
RECALL_INNER="$(printf '%s' "$RECALL_FRAME" | jq -r '.result.content[0].text')"
HITS="$(printf '%s' "$RECALL_INNER" | jq '.results | length')"
if (( HITS < 1 )); then
    echo "FAIL: recall returned 0 hits, expected >=1: $RECALL_INNER" >&2
    exit 19
fi
echo "    recall returned $HITS hit(s)" >&2

echo "PASS: deferred-sign flow round-trip succeeded (correlation_id=$CORRELATION_ID, attestation_id=$ATT_ID)"
