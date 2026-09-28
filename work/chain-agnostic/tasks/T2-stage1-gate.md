---
status: pending
wave: 1
blocked_by: [live-data-verification]
---

# T2: Acceptance gate — Irys superset of memo enumeration

Run the live parity check:
```bash
# Set a wallet address with known anchored memories
WALLET=<base58-pubkey>
GRAPHQL_URL=https://uploader.irys.xyz/graphql

# Enumerate via Irys
cargo run --example enumerate -- --graphql $GRAPHQL_URL --wallet $WALLET > irys.txt

# Enumerate via Solana memos  
cargo run --example enumerate -- --solana $RPC_URL --wallet $WALLET > memo.txt

# Verify superset
diff <(sort memo.txt) <(sort irys.txt)
```

Irys output must be a superset of memo output. Until it is, stage 2 does not start.
