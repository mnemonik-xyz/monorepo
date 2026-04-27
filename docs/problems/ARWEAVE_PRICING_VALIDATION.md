# Arweave Pricing Validation

**Date:** 2026-04-01
**Addresses:** CRITICAL_REVIEW.md Issue 1
**Script:** `validate_arweave_pricing.mjs`

---

## Results (Option A — API price query)

**Measured via Irys bundler price API + CoinGecko AR/USD:**

| AR/USD | Irys rate | Whitepaper claimed | Ratio |
|--------|-----------|-------------------|-------|
| $1.75 | **$16.74/GB** | ~$5.00/GB | 3.3x understated |

**Note:** Small uploads (<100KB) have a base fee that makes per-GB rate higher (~$137/GB at 10KB). Pricing is linear from ~100KB upward at $16.74/GB.

### Mnemonic snapshot costs (measured)

| Scenario | Snapshot size | Cost/snapshot | Cost if daily | Cost/month |
|----------|--------------|--------------|---------------|-----------|
| 1k memories, 384-dim | ~2 MB | $0.03 | $0.03/day | $1.00 |
| 1k memories, 1536-dim | ~8 MB | $0.13 | $0.13/day | $4.02 |
| 10k memories, 384-dim | ~15 MB | $0.25 | $0.25/day | $7.53 |
| 10k memories, 1536-dim | ~100 MB | $1.67 | $1.67/day | $50.21 |

### Impact on whitepaper economics

The whitepaper's $5/GB figure is **3.3x too low** at current AR price ($1.75). This affects:
- Section 5.1 storage cost table
- The "sub-$1 for 100K memories" claim in the conclusion (actual: ~$1.67 at 384-dim, ~$16.74 at 1536-dim)
- Comparison table vs Pinecone/pgvector (Mnemonic is still cheaper, but margins are tighter)

The per-snapshot costs are still economically viable for the target use case (researchers, not high-frequency traders), but the whitepaper must use accurate numbers.

---

## Validation Options (for future reference)

### Option A — API price query (DONE)
- **Method:** Query Irys price endpoint + CoinGecko AR/USD
- **Cost:** Zero
- **Script:** `validate_arweave_pricing.mjs`
- **Accuracy:** Current bundler rate; does not account for Irys free tier, discounts, or actual transaction fees
- **Status:** ✅ Completed 2026-04-01

### Option B — Real micro-upload test
- **Method:** Upload a small test blob (10KB–1MB) through `commit.mjs` (non-dry-run), measure actual cost charged
- **Cost:** ~$0.001–0.02
- **How to implement:**
  1. Generate a random 10KB blob: `node -e "require('fs').writeFileSync('test_10kb.bin', require('crypto').randomBytes(10000))"`
  2. Upload via `commit.mjs` with `--no-dry-run` flag
  3. Record tx ID, check Arweave explorer for actual cost
  4. Repeat at 1MB for a second data point
- **Accuracy:** Measures real cost through our actual pipeline, including any bundler markup or free tier behavior
- **When to run:** Before any external publication of economics claims
- **Status:** Not yet implemented

### Option C — Upload realistic snapshot
- **Method:** Upload actual snapshot files (`data/snapshot_1k_8bit.bin` ~2MB, `data/snapshot_10k_8bit.bin` ~15MB) via `commit.mjs`
- **Cost:** ~$0.03–0.25
- **How to implement:**
  1. Use existing `commit.mjs` pipeline: `node onchain/commit.mjs --blob data/snapshot_1k_8bit.bin`
  2. Record: blob size, Arweave tx ID, actual cost
  3. Verify: fetch blob from Arweave gateway, confirm hash matches
  4. Repeat with 10k snapshot for second data point
- **Accuracy:** Most accurate — real payload, real pipeline, real cost
- **When to run:** Before V1 SDK release; results go into ADR-017
- **Status:** Not yet implemented

---

## Recommended whitepaper fix

Replace `~$5/GB` with `~$17/GB (at AR=$1.75, via Irys bundler, April 2026)` and add a note that Arweave pricing is AR-denominated and varies with the AR/USD exchange rate.

Updated economics table should use the measured $16.74/GB rate and include a caveat:
> Arweave storage costs are denominated in AR tokens. The USD costs shown use the AR/USD rate at time of measurement ($1.75, April 2026). At AR=$10 (previous highs), costs would be ~$96/GB. At AR=$0.50, costs would be ~$4.78/GB.
