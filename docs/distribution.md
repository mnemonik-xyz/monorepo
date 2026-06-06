# Distribution & Discovery Playbook

How we advertise the Mnemonic MCP server and get it listed in the catalogues
agents and developers actually browse. This file is the single operator-facing
source of truth: every submission below has ready-to-paste copy so listing is a
mechanical step, not a writing exercise.

> **Framing rule (do not break):** lead every description with *utility*
> ("verifiable, persistent memory for AI agents"), not crypto. Smithery and the
> official registry are community-reviewed and heavy crypto framing has been
> flagged as a rejection risk (see `smithery.yaml` header + the integrations
> risk table). Solana/Arweave anchoring is a *feature we mention second*, never
> the headline.

---

## Canonical description (reuse verbatim)

**Two-sentence (README, registry long description, blog intros):**

> Mnemonic is a verifiable, persistent memory layer for AI agents, exposed over
> the Model Context Protocol (MCP). Every memory is semantically embedded,
> signed with your own Ed25519 identity, and optionally anchored on Solana and
> Arweave — so it stays portable across tools like Claude, Cursor, and VS Code
> and can be independently verified by anyone.

**One-line (≤100 chars, registry `description`, catalogue cards):**

> Verifiable, persistent memory for AI agents over MCP — signed, portable, on-chain anchored.

**Tagline (badges, social bios):**

> Verifiable memory for AI agents.

**Tags / keywords (reuse across listings):**
`memory`, `mcp`, `ai-agents`, `attestation`, `identity`, `knowledge`,
`context-portability`, `solana`, `arweave`, `verifiable`.

**Canonical links:**

| What | URL |
|---|---|
| Website | https://mnemonik.xyz |
| Install hub | https://mnemonik.xyz/install |
| Hosted MCP endpoint | https://mcp.mnemonik.xyz/mcp |
| Health | https://mcp.mnemonik.xyz/health |
| Repo | https://github.com/mnemonik-xyz/monorepo |
| npm (shim) | https://www.npmjs.com/package/@mnemonik-xyz/mcp |
| npm (cli) | https://www.npmjs.com/package/@mnemonik-xyz/cli |
| Discord | https://discord.gg/ws6wruJj |

---

## Pre-submission checklist

Run this once before submitting anywhere — several registries probe the
endpoint and a red check kills the listing.

- [ ] `curl -fsS https://mcp.mnemonik.xyz/health` returns 200.
- [ ] `initialize` + `tools/list` respond **without** a Bearer token (discovery
      must be pre-auth — registries crawl these). Quick check:
      ```bash
      curl -s https://mcp.mnemonik.xyz/mcp \
        -H 'content-type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
      ```
- [ ] OAuth metadata in `smithery.yaml` / `server.json` matches the live
      `/oauth/authorize` + `/oauth/token` routes.
- [ ] `@mnemonik-xyz/mcp` version in `server.json` matches the latest npm
      publish (`npm view @mnemonik-xyz/mcp version`).
- [ ] README "Add to your AI tool" deeplinks open the install dialog in a real
      Cursor + VS Code (regression-prone — see `InstallButtons.tsx` notes).

---

## Catalogue overview

| Catalogue | Method | Artifact | Gate / review | Status |
|---|---|---|---|---|
| **Smithery** | Web UI submit | `smithery.yaml` | community review | manifest ready — **submit** |
| **Official MCP Registry** | `mcp-publisher` CLI | `server.json` | automated + namespace auth | manifest ready — **publish** |
| **Glama** | Auto-crawl + claim | repo + `server.json` | automated | claim after crawl |
| **mcp.so** | Web form | listing copy below | light review | submit |
| **PulseMCP** | Web form / crawl | listing copy below | editorial | submit |
| **Cursor directory** | Web form | deeplink + copy | review | submit |
| **Anthropic Connectors** | Application | listing copy below | high-bar review | apply |
| **awesome-mcp-servers** | GitHub PR | one-line entry | maintainer review | open PR |

---

## 1. Smithery — `smithery.ai`

**Status:** `smithery.yaml` is committed at repo root and validated. The actual
submission is a manual web step (it was deferred to post-deploy in the
integrations spec — this is the step to do now).

**Steps:**
1. Sign in at https://smithery.ai with the `mnemonik-xyz` GitHub org.
2. **Add Server → from GitHub repo** → select `mnemonik-xyz/monorepo`.
   Smithery reads `smithery.yaml` at the repo root.
3. Confirm the endpoint `https://mcp.mnemonik.xyz` and the OAuth2 flow render
   correctly in the preview.
4. Submit. Once live the page is `https://smithery.ai/server/mnemonic` (or the
   slug Smithery assigns).
5. Grab the Smithery badge for the README (see Badges section).

**If review pushes back on framing:** the description already leads with
"verifiable knowledge memory". Do not add chain-first language to pass review.

---

## 2. Official MCP Registry — `registry.modelcontextprotocol.io`

The highest-trust listing; it is what MCP clients increasingly read
programmatically. Manifest is `server.json` at repo root.

**Steps:**
1. Install the publisher CLI: `npm i -g @modelcontextprotocol/publisher`
   (a.k.a. `mcp-publisher`).
2. **Validate / refresh the schema** — the registry schema is a moving target.
   Run `mcp-publisher init` in a scratch dir to see the *current* `server.json`
   shape, then reconcile our committed `server.json` against it (field names,
   `$schema` version). Our values (name, remote, npm package) stay; only the
   envelope may need a version bump.
3. Authenticate the namespace. We use `io.github.mnemonik-xyz/mnemonic`, which
   is owned via the GitHub org — `mcp-publisher login github` and authorize as
   `mnemonik-xyz`. (Alternative: a DNS-verified `xyz.mnemonik/...` namespace
   using a TXT record on mnemonik.xyz.)
4. `mcp-publisher publish` from the repo root.
5. Verify: `curl -s "https://registry.modelcontextprotocol.io/v0/servers?search=mnemonic"`.

**Owner action required:** step 3 needs an interactive GitHub auth as the org —
I cannot complete that from here. Everything up to it is prepared.

---

## 3. Glama — `glama.ai/mcp/servers`

Glama auto-crawls public GitHub repos with MCP servers.

**Steps:**
1. Wait for / trigger the crawl: search https://glama.ai/mcp/servers for
   "mnemonic". If absent, submit the repo URL via their "Add server" form.
2. **Claim** the server (GitHub OAuth as `mnemonik-xyz`) to edit the listing.
3. Set the description (one-line canonical), tags, and the hosted endpoint.
4. Glama scores repos on quality signals (license, README, tests, activity) —
   ours already has CI badges, Apache-2.0, and a full README, so no extra work.

---

## 4. mcp.so — `mcp.so`

High-traffic discovery directory.

**Steps:**
1. Open https://mcp.so/submit (or the "Submit" link in the nav).
2. Paste:
   - **Name:** Mnemonic
   - **Repo:** https://github.com/mnemonik-xyz/monorepo
   - **Description:** _one-line canonical_
   - **Endpoint:** https://mcp.mnemonik.xyz/mcp
   - **Tags:** memory, ai-agents, attestation, solana
3. Submit and watch for the listing (usually 1–3 days).

---

## 5. PulseMCP — `pulsemcp.com`

Directory + a widely-read weekly newsletter — good for a launch spike.

**Steps:**
1. https://www.pulsemcp.com/submit
2. Same fields as mcp.so. PulseMCP is editorial; include the install hub URL
   (https://mnemonik.xyz/install) so they can screenshot the one-click flow.
3. Optional: email their newsletter tip line referencing the launch post.

---

## 6. Cursor MCP directory — `cursor.com/directory` (a.k.a. cursor.directory)

**Steps:**
1. Submit at https://cursor.directory (community-run) and via Cursor's
   first-party directory form if open.
2. Provide the install deeplink (already in README):
   `cursor://anysphere.cursor-deeplink/mcp/install?name=Mnemonic&url=https%3A%2F%2Fmcp.mnemonik.xyz%2Fmcp`
3. Description: _one-line canonical_. Category: Memory / Knowledge.

---

## 7. Anthropic Connectors directory

Highest distribution for Claude users, highest review bar.

**Steps:**
1. Apply via Anthropic's connector/partner intake (developer docs →
   "Build a remote MCP server" → directory submission).
2. Requirements to have ready: hosted streamable-HTTP endpoint (✅), OAuth 2.1
   (✅), privacy policy + ToS URL, and a support contact (dev@mnemonik.xyz).
3. Lead with cross-tool portability and verifiability; this is the audience
   where "your memory follows you out of Claude into Cursor" lands hardest.

**Owner action:** needs a privacy policy + ToS page live on mnemonik.xyz before
applying. Flag if those don't exist yet.

---

## 8. awesome-mcp-servers (GitHub lists)

Cheap, fast, good SEO. Several lists exist; the most-starred is
`punkpeye/awesome-mcp-servers`.

**PR body (ready to paste):**

> Adds Mnemonic to the Knowledge & Memory section.
>
> **Mnemonic** — Verifiable, persistent memory for AI agents over MCP. Memories
> are semantically embedded, Ed25519-signed, and optionally anchored on Solana +
> Arweave, so they stay portable across Claude, Cursor, and VS Code and can be
> independently verified. Hosted endpoint + open-source (Apache-2.0).

**List entry (Markdown):**

```markdown
- [Mnemonic](https://github.com/mnemonik-xyz/monorepo) 🦀 ☁️ - Verifiable, persistent memory for AI agents — signed, portable, on-chain anchored.
```

Open the same PR against the other prominent lists
(`appcypher/awesome-mcp-servers`, `wong2/awesome-mcp-servers`).

---

## Launch posts (drafts)

Fire these on the **same day** the Smithery + registry listings go live, so the
"where do I get it" question has an answer.

**Hacker News (Show HN):**

> Show HN: Mnemonic — verifiable, portable memory for AI agents (MCP)
>
> AI agents forget between sessions, and when memory does persist there's no way
> to verify what the agent actually remembered. Mnemonic is an MCP server that
> gives agents a memory layer where every entry is semantically embedded, signed
> with your own Ed25519 key, and optionally anchored on-chain — so the same
> memory is recallable and verifiable across Claude, Cursor, and VS Code. Local
> mode runs fully offline; the hosted endpoint is one click from
> mnemonik.xyz/install. Open-source (Rust, Apache-2.0). Happy to answer
> questions on the canonical-CBOR + COSE_Sign1 design.

**Reddit (r/LocalLLaMA, r/mcp, r/ClaudeAI):** same hook, shorter, lead with the
offline-local angle for r/LocalLLaMA; lead with cross-tool recall for r/ClaudeAI.

**X / Twitter thread (5 posts):**
1. Hook: "AI agents forget. And when they remember, you can't verify what."
2. What Mnemonic is (two-sentence canonical).
3. The portability demo: sign in Claude, recall in Cursor (GIF).
4. The verifiability angle: signed + optionally on-chain anchored.
5. CTA: one-click install → mnemonik.xyz/install, open-source link.

**Discord / Telegram announcement:** short, link the listings + install hub +
ask for upvotes on the HN/Reddit posts.

**Content piece:** turn `docs/comparisons.md` into a blog post —
"Portable memory vs. per-tool memory" — and cross-link from every listing.

---

## Badges (for README once live)

Add to the badge row at the top of `README.md` after each listing goes live
(kept out until live so we don't ship 404 badges):

```markdown
[![smithery badge](https://smithery.ai/badge/mnemonic)](https://smithery.ai/server/mnemonic)
```

The Smithery badge URL/slug is confirmed at submission time (section 1).

---

## Status tracker

Update as each listing lands:

- [ ] Smithery submitted / live
- [ ] Official MCP Registry published
- [ ] Glama claimed
- [ ] mcp.so listed
- [ ] PulseMCP listed
- [ ] Cursor directory listed
- [ ] Anthropic Connectors applied / approved
- [ ] awesome-mcp-servers PR(s) merged
- [ ] Launch posts published (HN / Reddit / X / Discord)
- [ ] README badges added for live listings
