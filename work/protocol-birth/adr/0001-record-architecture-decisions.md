# 0001. Record architecture decisions

- Status: Accepted
- Date: 2026-10-10

## Context

Decisions about the Mnemonic protocol were spread over chat sessions, pull requests
and 27 feature decision logs in the monorepo. Several decisions on 2026-10-10 span
more than one repository.

## Decision

Record each protocol-level decision as an architecture decision record (ADR) in
`work/protocol-birth/adr/`, numbered in the order the decisions were made. Each record
has: status, date, context, decision, consequences and alternatives considered.

An accepted record is not rewritten. A later decision supersedes it in a new record,
and both records link to each other. Feature-level decisions stay in the feature's
own `decisions.md`.

## Consequences

- One place shows why the protocol looks as it does.
- Records reference the pull requests that implement them.

## Alternatives considered

- Only feature `decisions.md` files: hard to find decisions that span features and repositories.
- Only the brainstorm document: good for discussion, but not a stable record.
