# Dev tools — local Mnemonic round-trip

Helpers to stand up a local `mnemonic-mcp` and exercise sign → recall → verify
without the hosted endpoint. Reproduces the live verification from the paradigm
work. **No secrets are committed** — secrets are generated/ephemeral.

## One-time setup
```bash
sudo apt-get install -y libdbus-1-dev          # keyring dep (else libdbus-sys panics)
cargo build -p mnemonic-mcp --release --features local-embed
./fetch-fastembed-model.sh                      # caches the ONNX model (HF mirror)
```

## Run + auth
```bash
./run-local-mcp.sh                              # prints MCP_JWT_SECRET, serves :4000
# in another shell — mint a Bearer JWT for sign/verify (recall is open):
TOKEN=$(MCP_JWT_SECRET=<printed> node mint-jwt.cjs)
curl -s -X POST http://127.0.0.1:4000/mcp \
  -H 'Content-Type: application/json' -H "Authorization: Bearer $TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemonic_sign_memory","arguments":{"content":"hello","mode":"local"}}}'
```

## Full round-trip through the Arco proxy
The end-to-end harness (signed-challenge auth → Arco `/api/mnemonic/*` proxy →
MCP) lives in the **Arco-Agent** repo: `scripts/mnemonic-roundtrip.ts`
(`ARCO_URL=http://localhost:3000 npx tsx scripts/mnemonic-roundtrip.ts`).

See [`../IMPLEMENTATION_STATUS.md`](../IMPLEMENTATION_STATUS.md) for the full
recipe + gotchas.
