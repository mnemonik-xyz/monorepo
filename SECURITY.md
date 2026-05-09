# Security Policy

The Mnemonic Protocol team takes security seriously. Thank you for taking the time to disclose responsibly.

## Reporting a Vulnerability

Email **dev@mnemonik.xyz** with a clear description, reproduction steps, and impact assessment. Do not file public GitHub issues for security reports.

For sensitive disclosures we prefer encrypted email. A PGP key is **TBD** — request the current key by email and we will provide it before you send the report.

## Response SLA

- **Acknowledgment:** within **72 hours** of receipt.
- **Triage and initial assessment:** within 7 days.
- **Disclosure window:** **90 days** from acknowledgment, unless mutually agreed otherwise. We will coordinate a timeline for fix, release, and public advisory.

## Scope

In scope:

- Cryptographic flaws in our use of **COSE_Sign1**, **blake3**, or **Ed25519** (signing, verification, canonicalization, hash construction).
- Authentication or **JWT** validation bypasses in the SDK, CLI, or hosted services.
- Server-side bugs at **mcp.mnemonik.xyz** (auth, rate limiting, data exposure, injection, deserialization).
- Privilege escalation or arbitrary code execution in the **CLI** or **SDK** running on a user host.
- Supply-chain integrity issues in our published artifacts (npm, crates.io, Docker images).

Out of scope:

- Issues already tracked in public GitHub issues or release notes.
- Vulnerabilities in third-party dependencies — please report those upstream. We will pick up patched releases in our normal update cycle.
- Denial of service via volumetric traffic against public endpoints.
- Self-XSS, missing security headers without demonstrated impact, social engineering, or physical attacks.

## Safe Harbor

Good-faith research conducted under this policy will not result in legal action from us. Please avoid privacy violations, data destruction, and service degradation while testing.

## Credit

We credit reporters in release notes and the project changelog unless you request anonymity. Let us know your preferred name or handle in your initial report.
