# Phase 1 remediation tasks

These tasks implement the remediation plan in `../progress.md`. Complete them
in numeric order unless a task explicitly says otherwise.

```text
00 test-fixtures (complete)
        |
        v
01 durable paid operation ----> 04 secure approval/resume ----> 06 recovery tests
        |                                  ^                         |
        v                                  |                         v
02 canonical signed binding --------------+                    07 staging delivery
        |
        v
03 wallet-to-subject binding

05 exact-first capability policy is required before Phase 1 staging.

06 recovery tests (complete)
        |
        v
07a external-delivery staging gate -> 07b operational readiness -> 07c staging evidence
```

No task may change the legacy recall/verification path for already anchored
artifacts. Stake remains disabled for hosted Phase 1; its design is tracked by
the requirements in `../tech-spec.md` and follows only after exact hardening.
