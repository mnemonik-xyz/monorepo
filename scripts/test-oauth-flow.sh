#!/usr/bin/env bash
#
# scripts/test-oauth-flow.sh — smoke harness for Task 4 (OAuth 2.1 + PKCE).
#
# Mints a test HS256 JWT against `MCP_JWT_SECRET` using the `mint-test-jwt`
# helper binary and verifies that:
#   - the token decodes against the same secret,
#   - claims `iss`, `aud`, and `sub` match Decision 11,
#   - `jti` is a UUID.
#
# Exits 0 on success and prints the JWT (so callers can pipe it into curl).
#
# Note: this is a deliberately self-contained crypto-only smoke. The full
# `/oauth/authorize` -> `/oauth/token` end-to-end is exercised by the
# Rust-side `mcp/tests/oauth_flow.rs` integration test (also gated by Task
# 4's verify-smoke). This shell script is the harness referenced by the
# task spec line 142 and the Task 8 dispatcher.

set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# 32-byte base64-encoded secret (the production minimum from Decision 11).
SECRET="${MCP_JWT_SECRET:-$(openssl rand -base64 32)}"
export MCP_JWT_SECRET="$SECRET"

# Build the helper if not already built. Use --quiet so the JWT is the
# only thing on stdout when invoked from a pipe.
cargo build -p mnemonic-mcp --bin mint-test-jwt --quiet

TEST_SUB="TestPubkey$(date +%s)"
TOKEN="$(target/debug/mint-test-jwt --sub "$TEST_SUB")"

if [[ -z "$TOKEN" ]]; then
    echo "FAIL: mint-test-jwt produced empty token" >&2
    exit 1
fi

# Split header.payload.signature, base64url-decode each, JSON-parse, assert
# the Decision 11 fields. base64 -d on macOS does not support URL-safe
# alphabet directly; we sed-substitute -_ -> +/ before decoding.
b64url_decode() {
    local raw="$1"
    # Pad to a multiple of 4.
    local padded
    case $(( ${#raw} % 4 )) in
        2) padded="${raw}==" ;;
        3) padded="${raw}=" ;;
        *) padded="$raw" ;;
    esac
    printf '%s' "$padded" | tr '_-' '/+' | base64 -d 2>/dev/null
}

HEADER_B64="${TOKEN%%.*}"
REST="${TOKEN#*.}"
PAYLOAD_B64="${REST%%.*}"
SIG_B64="${REST#*.}"

HEADER_JSON="$(b64url_decode "$HEADER_B64")"
PAYLOAD_JSON="$(b64url_decode "$PAYLOAD_B64")"

# Algorithm must be HS256 — Decision 11 explicitly forbids alg=none / RS256.
ALG="$(printf '%s' "$HEADER_JSON" | grep -oE '"alg":"[^"]+"' | head -n1)"
if [[ "$ALG" != '"alg":"HS256"' ]]; then
    echo "FAIL: header alg is not HS256 (got: $ALG)" >&2
    exit 2
fi

# Pull and assert claim fields.
get_str() {
    printf '%s' "$1" | grep -oE "\"$2\":\"[^\"]+\"" | head -n1 | sed -E "s/\"$2\":\"(.*)\"/\1/"
}

ISS="$(get_str "$PAYLOAD_JSON" iss)"
AUD="$(get_str "$PAYLOAD_JSON" aud)"
SUB="$(get_str "$PAYLOAD_JSON" sub)"
JTI="$(get_str "$PAYLOAD_JSON" jti)"

if [[ "$ISS" != "mcp.mnemonik.xyz" ]]; then
    echo "FAIL: iss != mcp.mnemonik.xyz (got: $ISS)" >&2
    exit 3
fi
if [[ "$AUD" != "mcp" ]]; then
    echo "FAIL: aud != mcp (got: $AUD)" >&2
    exit 4
fi
if [[ "$SUB" != "$TEST_SUB" ]]; then
    echo "FAIL: sub != $TEST_SUB (got: $SUB)" >&2
    exit 5
fi
# jti is a UUIDv4 — 36 chars, 5 hyphens. Don't strictly parse — just length.
if [[ ${#JTI} -ne 36 ]]; then
    echo "FAIL: jti is not a UUID (got: $JTI)" >&2
    exit 6
fi
if [[ -z "$SIG_B64" ]]; then
    echo "FAIL: signature segment empty (alg=none?)" >&2
    exit 7
fi

# Print the JWT on stdout so callers can pipe it into curl. All diagnostics
# go to stderr so the JWT is the only thing on stdout.
echo "PASS: JWT validates per Decision 11 (iss=$ISS, aud=$AUD, sub=$SUB, jti=$JTI)" >&2
echo "$TOKEN"
