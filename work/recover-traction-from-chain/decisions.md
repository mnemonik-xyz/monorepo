# Decisions — recover-traction-from-chain

## 2026-07-09 — D1: chain snapshot + merge, not DB re-import

User explicitly preferred the GraphQL approach ("why double data"). The
chain snapshot lives in memory (`ChainStatsCache`), refreshed hourly; the
DB is never backfilled. Consequence: Ledger content/recall are NOT
recovered — only the public traction numbers. A future `reindex-from-chain`
command can reuse `snapshot_chain` if content recovery is ever wanted.

## 2026-07-09 — D2: union by `arweave_tx`, DB day wins

Anchored counts dedupe on the Arweave item id, which is identical in the
gateway index and the `attestations.arweave_tx` column. For items present
in both, the DB's `created_at` day is used (exact write time; block
timestamps lag by minutes and pending items have none).

## 2026-07-09 — D3: `Producer`/`Created-At` tags on new uploads

Both values are already public inside the COSE payload (webapp fetches it
freely from Arweave), so tagging adds zero privacy exposure while making
distinct-user aggregation a pure GraphQL operation. Legacy items are
backfilled once per process by decoding payloads.

## 2026-07-09 — D4: fail-fast on bad `CHAIN_STATS_WALLETS`

An invalid wallet aborts startup instead of logging: a typo would silently
shrink the public traction numbers, which is worse than a crash the
operator sees immediately.

## 2026-07-09 — D5: only the PUBLIC wallet address enters config

The feature needs no signing. Operator instruction: derive the address via
`solana-keygen pubkey <keypair.json>`; the private key stays in the
secrets store (SOPS planned by the operator) and must never appear in
`CHAIN_STATS_WALLETS`.

## 2026-07-09 — D6: Solana memo history is the primary enumeration source

Live verification against the production wallet
`DYVu4Bry3BzGVsR3Hj2iGVT5fNdWFoHw2zRxsdTmrG25` found 16 anchor memos
(2026-05-02 … 2026-05-20) but ZERO GraphQL hits on arweave.net and
Goldsky — even by exact id. The historical Irys-bundled items were never
indexed by the gateways. `SolanaClient::list_memo_anchors`
(`getSignaturesForAddress` + memo parse) now feeds `snapshot_chain`,
unioned with GraphQL by `arweave_tx`. Recovered live: 16 anchored
memories, 3 distinct users, all 16 payloads readable.

## 2026-07-09 — D7: payload gateway defaults to gateway.irys.xyz

arweave.net returns an HTML placeholder page with HTTP 200 for
Irys-bundled items it never indexed — a silent-corruption trap (COSE
decode fails, producer counts as None). The Irys gateway serves the
real bytes. Also: Irys's WAF 403s some default user agents
(`Python-urllib` blocked), so core's Arweave HTTP clients now send an
explicit `mnemonic-core/<version>` UA.

## 2026-07-09 — D8: wallet FkwN… is NOT the production wallet

The keychain backup the operator found
(`FkwNSXvgRVwxXWZDmvpxpYhvc1Zg5kbmx8NCtZxtLYAk`) has zero mainnet and
devnet history and zero balance. The production anchor wallet is the
documented VPS keypair `DYVu4Bry3BzGVsR3Hj2iGVT5fNdWFoHw2zRxsdTmrG25`
(deployment.md, `/home/claude/monorepo/keypair/id.json`) — 19 mainnet
txs, 16 anchor memos, ~0.22 SOL. `CHAIN_STATS_WALLETS` must be set to
the DYVu… address.
