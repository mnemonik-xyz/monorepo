# Backlog — `chrome-extension/` (Phase 1.5+)

Items deliberately deferred from Phase 1. Architecture must remain open for them but no implementation work happens until promoted to a task.

## Platform reach

- Firefox port (Manifest V3 in Firefox is now stable). Most code reusable; service-worker → background-script differences abstracted.
- Safari port (Xcode wrapper, `safari-web-extension-converter`). Embedder model size review (App Store limits).
- Edge / Brave / Arc — should work as-is on Chromium build; smoke-test before Web Store submission.

## More chat adapters

- Grok (`x.com/i/grok`)
- Perplexity (`perplexity.ai`)
- Poe (`poe.com`)
- OpenRouter playground (`openrouter.ai`)
- t3.chat
- Mistral Le Chat (`chat.mistral.ai`)
- DeepSeek chat
- Local-LLM web UIs (Ollama UI, LM Studio web export, OpenWebUI)
- Cursor / Windsurf web (when shipped)

Each = new adapter module + HAR fixture + registry entry. Decision D10 makes this a small, isolated change.

## Capture / recall enhancements

- `findInputBox` impl for Claude.ai + Gemini + others (Phase 1 ships only ChatGPT). Enables "Insert into chat" recall on all platforms.
- Auto-capture: per-domain background watcher that incrementally builds a draft attestation and signs at session end / user click. Off by default (D12).
- Inline floating "Save this turn" button next to each assistant message (instead of FAB only).
- Smart tagging: extract `model:`, `chat_id:`, `topic:` from page meta automatically.
- Multi-turn snapshotting (snapshot every N turns automatically as separate attestations).
- Diff between attestations of the same chat over time (was this conversation edited later?).
- Attach screenshot of chat at capture time (proof-of-rendering).

## Identity & auth

- WebAuthn / passkey wrap (replace Argon2id passphrase) — better UX, no passphrase to remember, but requires per-device passkey enrollment story.
- Apple Sign-in alongside Google.
- "Sign in with Solana wallet" alongside Google (for power users; bridge to existing webapp Solana OAuth).
- Multi-account profiles (`@work`, `@personal` keypairs).
- Hardware-key (Yubikey/Ledger) signing path.
- Recovery via Shamir threshold (3-of-5 social recovery, optional alternative to passphrase).

## Cloud / sync

- E2EE attestation content (encrypt payload before upload; recall does encrypted vector search via [Tahoe-LAFS-style] partial decrypt).
- Selective sync (per-tag sync rules; e.g. `tag:work` syncs to cloud, `tag:personal` stays local).
- Conflict resolution UI (when two devices add attestations offline).
- Merkle proof of cloud-hosted attestation set (server publishes daily merkle root on Solana; extension verifies).
- Incremental sync (delta protocol; not full re-pull on restore).

## Recall UX

- "Smart suggest" — when typing in any AI chat input, popup hint "you have 3 related memories" → click to insert.
- Memory chains (lineage edges) visualized as a graph in popup.
- Markdown rendering in recall preview (not just raw text).
- Filter by source platform (`only ChatGPT`, `only Claude`).
- Time-decay reranking option.

## Distribution

- Chrome Web Store featured listing application.
- Microsoft Edge Add-ons store submission (Manifest V3 compatible).
- Self-hosted `.crx` for enterprise (sideload distribution).
- Update channel (stable / beta).
- Telemetry pipeline (opt-in, anonymous adapter-broken / cold-start / sync-error metrics).

## Developer / extensibility

- Public `ChatAdapter` SDK for community-contributed adapters (separate npm package).
- Extension-to-extension messaging API so other extensions can `signMemory(content)` through Mnemonik.
- Programmatic recall API (other extensions query Mnemonik for context).
- MCP-from-extension (extension hosts a tiny MCP-stdio bridge for IDE integrations).

## Performance

- Replace `transformers.js` MiniLM with quantized INT8 build (~10MB) once it ships.
- Replace `442KB` WASM with `@noble/curves` + custom CBOR (already in `work/mnemonic-cli/backlog.md`); shrink popup cold-start.
- Pre-warm embedder on extension install (idle service-worker download).
- IndexedDB compaction on `navigator.storage.persist` quota pressure.

## Compliance / trust

- SOC2 / GDPR data-handling addendum for cloud-tier (when user count justifies).
- Privacy-policy generator that adapts to active permissions.
- "Data export" one-click (download all attestations as `.zip` of COSE bundles + .md).
- "Right to erasure" — DELETE all data on server + local on user request.
