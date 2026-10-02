# Product, sales and funding plan

Status: hypotheses and proposed experiments. Prices and customer demand are not validated.

## Buyer and offer

First segment: teams operating long-lived agents across replaceable runtimes.
Buyer: engineering or platform lead accountable for agent continuity.
User: agent developer; API caller: agent or runtime integration.
Problem: important history disappears or cannot migrate independently of one operator database.

Offer: save signed memory and recover it in a fresh runtime using customer-held trust anchors and keys.
The first demo deletes the local index, disables MCP, and restores an externally delivered chain.
Then it switches operator and continues the verified history.
Show provider errors and missing parents during the same demo.

Second segment: multi-agent teams that need verifiable authorship and context handoffs.
Do not market signature verification as proof of correct reasoning.
Personal users follow when backup and recovery are simple.
Enterprise work follows retention, key custody and counsel review.

## Packaging

| Package | Customer receives | Revenue basis | Evidence before sale |
|---|---|---|---|
| Open local tools | SDK, formats, verifier and offline local storage | Free | Offline conformance |
| Managed delivery | Supported upload, verified locator, operation status and retry | Usage fee plus disclosed external cost | Real end-to-end delivery and billing tests |
| Team continuity | Managed delivery, recovery guidance, monitors and incident support | Subscription plus usage | Customer recovery drill and measured support cost |
| Multi-operator routing | Compatible operator choice and measured failover | Optional coordination fee | Independent operators and observed switching value |

Do not charge merely because an artifact uses the open format.
A customer can use another operator or verify without Mnemonik login.
Discovery and open from known locators need a free independent path.
Managed recovery tools can add support, monitoring and convenience around that path.

## Unit economics

Separate storage cost, gateway cost, compute, payment cost, support and service margin.
Report free-quota subsidy separately from paid demand.
Track revenue per paying team, contribution margin and repeat recovery use.
Cheap writes alone may not cover integration and support costs.
Test whether a team package fits that cost better than a write-only fee.
Set prices after observing pilot costs; this document chooses no price or router percentage.

## Demand experiment

Interview ten target teams as an initial research budget, not a launch gate.
Ask about their latest memory loss or migration, current workaround and its cost.
Ask who approves spending and which data cannot enter permanent storage.
Recruit three design partners for the same narrow recovery scenario.
Ship one integration before expanding integrations.

Proposed pilot evidence: three paying teams and two repeated independent recovery drills.
These numbers are planning hypotheses. The owner may revise them from evidence.
Record refused offers and reasons, not only positive feedback.
Stop or narrow the product if teams do not experience the problem or will not pay.

## Funding and governance choices

| Question | Alternatives and dependency | Recommendation |
|---|---|---|
| Native token | No token; revisit after network demand; immediate issuance | No token work in the product roadmap |
| Fee allocation | Operator margin; accounting reserve; on-chain router | Start with transparent accounting and an incident reserve; measure costs first |
| Legal wrapper | Company; foundation; both | Counsel chooses jurisdiction and structure; engineering does not choose legal status |
| Home chain | Existing payment rails; one future token chain | Keep payment rails modular; defer token home chain |
| Governance | Maintainer process; contributor council; token votes | Publish maintainer decisions and spending controls; defer token votes |
| Gates | Fixed token thresholds; product release evidence | Use delivery, recovery and customer evidence; token thresholds remain dormant |
| Outside finance | Customer revenue; grants; equity; token sale | Test customer revenue; evaluate grants or equity by runway and obligations; no token-sale dependency |

For funds custody, define named accountable signers, spending limits and an emergency procedure.
Choose the mechanism after budget and entity decisions.
No unsigned business proposal in this package becomes an owner-approved treasury allocation.

## Counsel boundary

Counsel must review permanent storage, personal data, deletion claims and cross-border processing.
Counsel must review service terms, refunds, liability and availability commitments.
Counsel must review grants, fundraising instruments, entity choice and tax treatment.
Engineering supplies data-flow facts, retention facts and technical limits.
Engineering cannot declare legal compliance or classify a future token.
Do not publish an availability agreement before measured service capabilities and counsel review.
