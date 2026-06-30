---
created: 2026-06-30
updated: 2026-06-30
status: draft
type: business-model
relates: ./design.md
---

# Business model — who we sell to, and why "open protocol" is the wedge, not the problem

## Thesis

**Open protocols never monetize directly; the operational guarantee on top does.**
TCP/IP earned no one a cent — Cloudflare/AWS/ISPs monetized services on it. SMTP
is free; Gmail/SendGrid monetize it. Mnemonic's open protocol is the **adoption
wedge**; the **revenue is operating the guarantee** the protocol makes valuable:
proving + durable custody + audit-grade availability SLA.

The fear "it's just a protocol, no storage guarantee, no business model" is
backwards: **the storage/proving guarantee IS the product.** The protocol +
durability-class work (`design.md`) is the product's *engine*; the business is an
SLA + dashboard + sales motion wrapped around it.

## Why open is *required* here (not charity)

An audit/provenance format only has value if a regulator/auditor can verify it
**without trusting the vendor**. A closed, proprietary audit proof is a
non-starter for compliance — independent verifiability is the whole point.

So **open = credibility = the wedge.** This is also the structural win over Delta
(closed, hosted verifier → a regulator must trust Delta; ours is an open library
anyone runs). Open isn't generosity; it's the only way the product is *sellable*.

## Who buys

| Buyer | Pain | Why they pay |
|---|---|---|
| **Regulated enterprises running AI agents** (banks, insurers, healthcare — EU AI Act / FINRA / SOX / HIPAA) | must *prove* agents acted within policy or face fines / liability | **primary market**: budget + a deadline (EU AI Act, Aug 2026) + existential risk. Buy audit-grade proof + durable custody as a managed service. |
| **AI agent platforms / frameworks** | need to sell "compliant / auditable" to the above | embed the SDK; we are infrastructure (B2B2B) |
| **Auditors / GRC / compliance vendors** (Big Four, GRC SaaS) | run the actual audits | consume / white-label verification + dashboards |

## How we charge

1. **Managed proving + durability service** — per-proof / per-GB-permanent /
   subscription with an availability SLA. This is **D1+D3 relay-as-a-service**
   (instant accountable custody, batched to permanent). The core product.
2. **Compliance SaaS on top** — policy authoring, audit dashboards,
   regulator-ready export, retention management. Per-seat / enterprise.
3. **Open-core** — protocol + verifier Apache-2.0 (already); enterprise features
   (SSO, multi-tenant, support, SLAs, certifications) commercial. Standard
   OSS-infra playbook.
4. **(Deferred) incentivized relay network + token** — relays earn for honest D1
   custody. Higher regulatory/complexity risk; "business much later," not v1.

## Moat (honest)

It's open → anyone *can* self-host a prover + relay. The moat is therefore
operational, not secrecy:

- **Durability network** + availability SLA at scale.
- **Compliance certifications** (SOC2 / ISO 27001 / eIDAS) enterprises require.
- **Default-standard / trust** — being the format auditors already accept.
- **Proving-at-scale** economics.

This is exactly the moat of every open-core infra company. It works; it's earned
through operations and trust, not a closed codebase.

## Sequencing

`protocol now, business later` stands — but "later" is concrete: **operate the
guarantee.** v1 engineering (protocol + zigz proving + D1/D3 durability) *is* the
product engine. The business layer (SLA, dashboard, GTM) wraps it, aimed at the
EU-AI-Act-deadline buyer.

## The next decision: pick a design-partner vertical

The sharpest open question. v1 should aim at **one** concrete regulated vertical,
because that choice dictates which policies and evidence sources we build first:

| Candidate | First policies | Evidence source | Forcing function |
|---|---|---|---|
| **Agentic payments** (FINRA/PSP) | spending cap, allowlist, best-execution | zkTLS over merchant/exchange | agent commerce is live now (AP2/Delta heat) |
| **Healthcare** (HIPAA) | dose limits, allergy/interaction, prescriber auth | signed EHR/FHIR, e-prescribing | PHI privacy → ZK is the unlock |
| **AI training-data governance** (EU AI Act/GDPR) | consent-present, opt-out non-membership, licensed-source | signed consent receipts | EU AI Act Aug 2026; Mnemonic's own lane (provenance) |

**Recommendation:** lead with **agentic payments** (live market, the
Delta/AP2 momentum proves demand, evidence via zkTLS is well-trodden) OR **AI
training-data governance** (most aligned with Mnemonic's provenance identity and
the Aug-2026 deadline). Pick one; it shapes Wave 2's first guest program.

## Open questions

1. Which design-partner vertical for v1? (payments vs healthcare vs data-governance)
2. Commercial entity / licensing posture now, or defer until a design partner signs?
3. Is the incentivized-relay token path ever on the roadmap, or permanently out?
