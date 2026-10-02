# Multi-agent continuity proof of concept

Status: local synthetic implementation and recorded evidence are available in the
[demo guide](../../docs/research-handoff-demo.md). Live-provider acceptance remains open.
Related: [technical contract](tech-spec.md), [acceptance](acceptance.md),
[A2A recovery](../sealed-memories/a2a-recovery-spec.md).

## 1. Claim and boundary

Mnemonik enables independently verifiable memory handoffs and recovery across runtimes and operators.
The receiver verifies author, integrity, access grants and signed ancestry without trusting operator SQL.
SQL means Structured Query Language; here it identifies the operator's local database.
This removes the operator database from the memory trust boundary.

The receiver still needs independently trusted author keys, decryption keys and an authenticated recovery checkpoint.
External providers must supply the required bytes.
Signed statements can be false. Verified ancestry does not prove correct reasoning or complete execution.
Completeness applies to pinned heads, not every possible undisclosed artifact.

## 2. Recommended real-life scenario

Use a research project handed between independently operated specialist agents.
A project owner commissions a source collector and a reviewing agent.
Each agent can run with a different provider or in a replaceable container.
A replacement reviewer must recover the project without the collector's private database or the original Mnemonik operator.

This scenario makes authorship, handoff, confidentiality and continuity visible.
Use synthetic source material for the demo.
Exclude credentials, personal records and licensed full texts.
A real deployment needs a data-retention policy and counsel where required.

| Candidate | Fit | Limitation |
|---|---|---|
| Research handoff across agent providers | Strong first demonstration; clear signed source notes and review lineage | Signatures do not establish source truth |
| Incident investigation handoff | Strong operational value and recovery need | Sensitive logs and deletion requirements complicate permanent storage |
| Software-agent work handoff | Clear buyer and reproducible output | Git already supplies much artifact history; show missing agent context explicitly |
| Personal assistant migration | Clear continuity story | Less multi-agent interaction; identity and backup usability dominate |

The recommendation is a demonstration choice, not validated market demand.
The initial buyer hypothesis remains a team operating long-lived agents.

## 3. Roles

| Role | Owns or does | Trust boundary |
|---|---|---|
| Project owner | Chooses agents, pins keys, backs up checkpoint and controls budget | Defines trusted participants outside discovery |
| Collector A | Signs source notes and seals the collection for B | Responsible for its claims |
| Reviewer B | Verifies A, opens allowed memory and signs its review | Does not trust upload operator authorship |
| Replacement B | Runs B's role in a fresh environment | Uses securely restored B keys in this demo |
| Operator O1 | Verifies and delivers unchanged bytes; returns receipts | Holds neither author keys nor plaintext |
| Operator O2 | Delivers later continuation with empty receipt history | Verifies external parent instead of O1 SQL |
| Storage and index providers | Supply bytes and discovery candidates | Untrusted candidates; availability dependency |
| Outsider X | Attempts unauthorized open and forged imports | Must fail cryptographic checks |

The project owner supplies A and B keys through an authenticated setup.
Discovery must not select trusted authors for the owner.
Restoring B's key is identity continuity; provisioning a new C key requires explicit trust and grant policy.
A read grant is not general authority to mutate a project.
Use only the existing A2A continuation eligibility rule.

## 4. Artifact story

| Artifact | Author | Contents | Signed relationship |
|---|---|---|---|
| R | A | Research question and collection scope | Root |
| S | A | Completed sealed source bundle; B is a named recipient | Parent R |
| V | B | Review notes, uncertainty and next questions | Parent S |
| W | B in fresh runtime | Follow-up review after recovery | Parent V |

The source bundle includes a completed chunk stream to exercise full sealed recovery.
Use existing A2A signed bindings and hash meanings.
Do not introduce a new artifact format for the demo.
Author-signed grants let B open the required sealed artifacts.
A stores these grants before the handoff; O1 cannot manufacture them.

The owner independently preserves expected authors, head V, known locators and checkpoint authentication.
Secure key backup stays separate from external ciphertext and operator receipts.
The owner pays for delivery; payment does not confer authorship or read permission.

## 5. Demonstration flow

1. Configure A, B, expected identities and supported external endpoints.
2. A constructs, encrypts and signs R and S locally.
3. O1 validates and delivers exact signed bytes.
4. B fetches S, verifies A and the grant, then opens it locally.
5. B verifies the parent relationship and produces signed review V.
6. O1 delivers V; the owner independently saves the checkpoint to V.
7. Delete A2A receipts and reader rows from O1.
8. Destroy B's runtime, local index and sessions. Disable O1.
9. Start fresh B with securely restored keys and the independent checkpoint.
10. Discover candidates externally, fetch original bytes and verify R, S and V.
11. Rebuild ancestry, retain forks and open the complete source bundle locally.
12. B signs W with parent V and supplies V's external locator to O2.
13. O2 verifies V externally and delivers W without O1 access.

Do not delete financial state to demonstrate memory independence.
Stop claims of completeness at the checkpoint V.
W becomes another independently saved checkpoint after verified delivery.

## 6. Required failure demonstrations

| Injection | Required observation |
|---|---|
| Change artifact bytes | Signature or binding failure; no import |
| Repair public hashes after ciphertext tampering | Authenticated decryption or signed-binding failure |
| Index returns forged author or context tags | Reject candidate after byte verification |
| Outsider fetches ciphertext | Fetch may work; open must fail |
| Index omits a required parent | Missing parent reported; no complete-to-heads claim |
| Index returns no results during outage | Source error; no empty-success claim |
| Index has not caught up | Known locator can work; scan freshness remains uncertain |
| Valid fork appears | Preserve fork; no invented single canonical history |
| O1 cannot write a receipt after delivery | Preserve verified external result and report receipt failure separately |
| Payment settles but delivery fails | Separate states and visible remedy; retry does not charge again |

## 7. Demo screen and evidence

Use four views: project timeline, artifact inspection, failure controls and recovery status.
The timeline shows verified author, parent, artifact hash and locator.
Inspection shows signature and grant checks without showing secret keys.
Recovery shows pinned heads, verified graph, missing parents and source diagnostics.
Billing shows delivery and payment separately.

Before the crash, show R, S and V.
After the crash, show empty local state and disabled O1.
After recovery, show the same verified IDs, bytes and authorized plaintext.
After switching operators, show W linked to externally verified V.

Record exact commands, implementation commit, test result and provider configuration.
Record mocked integration and live-provider evidence separately.
Two local operator processes prove protocol behavior, not an independently operated production network.
Do not display a mocked result as live recovery.
Do not publish sensitive plaintext in evidence.

## 8. Implementation ownership

Reuse product tasks 2–5 and sealed-memory tasks 18–21.
[Task 8](tasks/8.md) assembles this demonstration after those capabilities pass.
The demo must cover acceptance A-2 through A-13 where applicable.
Customer validation A-14 remains separate.
No issue closes merely because this scenario is documented.
