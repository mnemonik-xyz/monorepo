# 0004. Read failover across Arweave gateways

- Status: Accepted, implemented (monorepo #279)
- Date: 2026-10-10

## Context

A live check found 2 of 5 public gateways failing at the same time (g8way.io did not
connect; permagate.io answered HTTP 503). Reads used one gateway with no retry, so one
refusal failed recovery, restore and delivery checks.

## Decision

Reads try the primary gateway (`ARWEAVE_GATEWAY_URL`), then the fallbacks
(`ARWEAVE_FALLBACK_GATEWAYS`, default arweave.net, ar-io.dev, turbo-gateway.com).
Each gateway gets 3 attempts for connection errors, HTTP 429, 5xx and empty bodies.
HTTP 404 moves to the next gateway at once. A read reports "not found" only when every
gateway answers 404.

## Consequences

- One failing gateway no longer breaks reads.
- Gateways are not trust anchors: clients still verify signatures and digests.

## Alternatives considered

- One gateway with retries: does not help when that gateway is down.
