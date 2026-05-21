# Decisions Log: memvid-backend

Append-only. This feature was evaluated and **rejected** before any user-spec or tech-spec was drafted. This file is the audit trail of that decision so future readers find an explicit "we considered, here's why we didn't" rather than empty silence.

---

## 2026-05-21 — Memvid backend rejected as a Mnemonic storage / portability feature

**Status:** Rejected (will not be built)
**Authors:** main agent + user
**Branch:** claude/analyze-portable-agent-memory-nYOJm

**Summary:** Considered adding a `MemvidStore` alongside the existing SQLite `AttestationStore`, plus `mnemonic export --memvid` / `mnemonic import` commands, to enable "shareable video file" portability of agent memory (built on `mnemonik-dev/memvid`). After enumerating 20 design considerations, the decision is to **not build this feature** in the current roadmap.

**Key considerations that drove rejection:**

1. **Per-file data ceiling is restrictive.** QR Version 40 ≈ 3 KB/frame; at 30 fps ≈ 5 MB/min raw. Practical mp4 sweet spot ≈ 500 MB → only ~100 MB of attestation bytes per file. Sharding is mandatory for any realistic user.
2. **Memvid is a UX/portability feature, not a storage optimization.** The resulting `.mp4` is materially larger than equivalent SQLite + Arweave bytes. The novelty pitch ("watch your memory like a movie") would land but does not justify the integration cost.
3. **Random access is intolerable.** Recall over a memvid file would require decoding every frame; the only viable path is "bulk-decode once → repopulate SQLite → recall as usual", which means memvid never substitutes for the primary store — it only duplicates work.
4. **Codec determinism vs cryptographic integrity.** Two encoders produce different mp4 bytes for the same input. We'd have to hash the canonical CBOR payload before encoding and treat the mp4 file hash as informational only — a constant source of confusion for users who expect "the file IS the proof".
5. **Lossy codecs corrupt QR.** Mandating CRF 0 (lossless) breaks the "playable on iOS/Android/desktop" appeal that's the entire point of the format.
6. **Memvid library risk.** Treating `mnemonik-dev/memvid` as a load-bearing dependency adds an external maintenance surface for limited downstream value.
7. **Append-only writes don't fit mp4.** A "hot SQLite + periodic batch-export" hybrid is the only sane write path, which means memvid is always derivative of SQLite — never authoritative.

**Concrete value proposition that did not survive the limitations review:**

- "Send your AI memory as a video file" — possible but realistic file sizes (sharded, encrypted, manifest-headed) make it functionally equivalent to a compressed tarball with extra steps.
- "Verifiable persistent memory in a portable format" — already delivered by COSE_Sign1 + canonical CBOR + Arweave; memvid adds no cryptographic guarantee.
- "Cool demo" — true, but not load-bearing for the protocol's core thesis.

**Alternatives that remain viable:**

- **Export to a simple tarball or zip** of canonical-CBOR + COSE_Sign1 artifacts. Same portability story without codec / QR / mp4 complexity. Could be a 1-day feature.
- **Per-user S3/IPFS export** with a recipient-bound encryption envelope. Standard tooling, no new abstractions.
- **Status quo:** existing Arweave anchoring already provides off-chain durability; webapp + CLI already provide cross-surface portability.

If any of the above becomes a roadmap priority, it gets its own feature directory (`work/portable-export/` or similar). This rejection does not preclude revisiting memvid in the future if (a) the upstream library matures significantly, or (b) a specific use case appears that genuinely benefits from the video-as-storage format.

**Deviations:** None — this is the rejection decision itself.

**Verification:**
- No code written; `work/memvid-backend/` contains only this `decisions.md`.
- 20-row considerations table archived in session transcript on 2026-05-21.
- No `MemvidStore` references introduced in `core/`, `mcp/`, or `packages/*/`.

**Follow-ups:**

1. If the "portable memory file" UX is later prioritized, draft `work/portable-export/` with the tarball/zip alternative as the default approach.
2. If `mnemonik-dev/memvid` is referenced from documentation, marketing, or whitepaper drafts, remove or replace those references to avoid implying integration.
3. Re-evaluate annually — codec landscape may change.
