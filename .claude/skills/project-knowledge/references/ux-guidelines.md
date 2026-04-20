# UX Guidelines

## Purpose
UX standards and user-facing communication for AI agents. Helps agents write consistent UI text and follow design patterns.
Applies to `webapp/` only. `core/` and `mcp/` have no UI.

---

## Interface Language

**Primary language:** English

**Localization:** English-only for MVP. Post-launch: extend to additional languages via i18n. No i18n framework configured yet — no need to extract strings into translation keys during MVP development.

---

## Tone of Voice

**Overall tone:** Technical / Precise

**Writing style:** Short, direct sentences. Active voice. No filler words or marketing language. Assume a developer audience that understands cryptographic concepts. Prefer exact terms over approximations ("attestation" not "proof of memory", "Ed25519 keypair" not "your identity").

**Voice characteristics:**
- **Formality level:** Professional. No contractions in UI labels. Conversational only in empty states.
- **Emotional tone:** Neutral and factual. No cheerful affirmations.
- **Technical complexity:** High — audience is developers. Do not simplify domain terms.
- **Humor:** None.

**Example phrases by context:**

- ✅ Good: "Memory attested on Arweave. Solana anchor: `5xQ3...`"
- ✅ Good: "Recall returned 0 results."
- ❌ Avoid: "Great news! Your memory has been saved!"
- ❌ Avoid: "Oops, something went wrong."

---

## Domain Glossary

- **Attestation** — a signed, on-chain-anchored memory record. Use "attestation", not "memory proof" or "entry".
  *UI example: "Attestation created · arweave_tx: `abc...`"*

- **Recall** — semantic search over stored attestations. Use "recall", not "search" or "find".
  *UI example: "Recall · top 5 results"*

- **Local mode** — SQLite-only, no blockchain. Use "local mode", not "offline" or "free mode".
  *UI example: "Running in local mode — attestations are not on-chain."*

- **Full mode** — Arweave + Solana + SQLite. Use "full mode", not "on-chain mode".

- **Agent identity** — the Ed25519 keypair that signs attestations. Use "agent identity" or "keypair", not "account" or "wallet".

---

## Text Patterns

### Buttons
**Style:** Action verb + object when space allows; single verb for icon buttons.

**Examples:**
- Primary: "Sign memory", "Export context", "Verify"
- Secondary: "Cancel", "Copy"
- Destructive: "Clear local store"

### Error Messages
**Format:** State the problem precisely. Include the relevant identifier if known. No apologies.

**Examples:**
- Validation: "Content must not be empty."
- Auth / keypair: "Keypair file not found at `~/.mnemonic/id.json`."
- Network: "Arweave upload failed (HTTP 503). Retry or switch to local mode."
- Verification: "Verification failed: content hash mismatch."

### Success Messages
**Format:** Confirmation + key identifiers. No exclamation marks.

**Examples:**
- "Memory attested · solana_tx: `5xQ3...` · arweave_tx: `kL9m...`"
- "Verification passed."
- "Context exported."

### Loading States
**Style:** Present continuous, lowercase, no ellipsis animation in text (use spinner component).

**Examples:**
- "Signing memory..."
- "Uploading to Arweave..."
- "Recalling..."

### Empty States
**Style:** One factual sentence describing the state. One short action prompt.

**Examples:**
- "No attestations yet. Sign a memory to get started."
- "Recall returned no results for this query."

---

## Design System

**Design files:** No Figma — built directly in Tailwind CSS.

**Color palette:**
- Background: `#0A0F1E` (deep navy-black)
- Primary accent: `#00D4B4` (teal/cyan)
- Secondary accent: `#9945FF` (Solana purple)
- Text primary: `#FFFFFF`
- Text muted: `#8B9BC0`
- Error: `#FF4747`
- Success: `#00CC88`

**Key components:**
- Dark card surface over `#0A0F1E` background
- Monospace font for all hashes, tx IDs, public keys, and code
- Minimal borders — subtle `#8B9BC0` at low opacity
- No heavy drop shadows — flat with subtle glow on primary accent elements

---

## Accessibility

Follow standard WCAG 2.1 AA guidelines. Ensure all hash/tx displays have `aria-label` with a human-readable description (e.g., `aria-label="Solana transaction ID"`).
