---
feature: arc-mainnet-deploy
created: 2026-09-27
status: draft
---

# Arc Mainnet — Target Chain Spec

Arc (Circle's native USDC L1) is the first chain where Mnemonik agents will submit
on-chain ERC-8004 reputation feedback. It went live **2026-09-16**. USDC is the gas
token, finality is sub-second, and validators include BlackRock, Visa, and Mastercard.

This document is the canonical reference for every contract address and chain parameter
needed to deploy and operate Mnemonik on Arc mainnet.

---

## 0. Complete Contract Address Reference

### ERC-8004 Registries (deployed via CREATE2 — confirmed on Arc mainnet)

| Contract              | Mainnet address                              | Testnet address                              |
|-----------------------|----------------------------------------------|----------------------------------------------|
| Identity Registry     | `0x8004A169FB4a3325136EB29fA0ceB6D2e539a432` | `0x8004A818BFB912233c491871b3d84c89A494BD9e` |
| Reputation Registry   | `0x8004BAa17C55a88189AE136b182e5fdA19dE9b63` | `0x8004B663056A597Dffe9eCcC1965A193B7388713` |

Both confirmed live on Arc mainnet (chain 5042) at standard CREATE2 addresses.
Source: [qntx/erc8004 networks.rs](https://github.com/qntx/erc8004).

### Circle — Core Stablecoins

| Token  | Mainnet address                              | Testnet address                              | Notes                         |
|--------|----------------------------------------------|----------------------------------------------|-------------------------------|
| USDC   | `0x3600000000000000000000000000000000000000` | `0x3600000000000000000000000000000000000000` | System contract, same address |
| EURC   | `0xbEf5f6d51CB62b58e6A8f77868681825C6fe21c1` | `0x89B50855Aa3bE2F677cD6303Cec089B5F319D72a` |                               |
| USYC   | `0x8a5D989Bbb96929F689B0200f435f53dA42bF490` | `0xe9185F0c5F296Ed1797AaE4238D26CCaBEadb86C` | Yield-bearing stablecoin      |

### Circle — Wrapped Assets

| Token   | Mainnet address                              | Testnet address                              |
|---------|----------------------------------------------|----------------------------------------------|
| cirBTC  | `0x171A4217b86A807A64eB94757Db6849fb4bDbAA0` | `0xf0C4a4CE82A5746AbAAd9425360Ab04fbBA432BF` |
| WETH    | `0x128cC466B61f542da60c70e3aA11c10e19B84EDB` | `0x2c4047028a72803939b6fb674D01bC059B5C4961` |

### Circle — CCTP v2 (Domain 26)

| Contract               | Mainnet address                              |
|------------------------|----------------------------------------------|
| TokenMessenger         | `0x28b5a0e9C621a5BadaA536219b3a228C8168cf5d` |
| MessageTransmitter     | `0x81D40F21F12A8F0E3252Bccb954D722d4c464B64` |
| CrossChainTokenService | `0x431871229103b780868f8C6BB820cd16ECf942BC` |

### Circle — Gateway (Programmable Wallets)

| Contract      | Mainnet address                              |
|---------------|----------------------------------------------|
| GatewayWallet | `0x77777777Dcc4d5A8B6E418Fd04D8997ef11000eE` |
| GatewayMinter | `0x2222222d7164433c4C09B0b0D809a9b52C04C205` |

### Standard EVM Infrastructure (same on mainnet and testnet)

| Contract        | Address                                      |
|-----------------|----------------------------------------------|
| Permit2         | `0x000000000022D473030F116dDEE9F6B43aC78BA3` |
| Multicall3      | `0xcA11bde05977b3631167028862bE2a173976CA11` |
| CREATE2 Factory | `0x4e59b44847b379578588920cA78FbF26c0B4956C` |

---

## 1. Arc Chain Parameters

| Parameter      | Mainnet                          | Testnet                               |
|----------------|----------------------------------|---------------------------------------|
| Network name   | Arc                              | Arc Testnet                           |
| Chain ID       | **5042** (`0x13b2`)              | **5042002**                           |
| CAIP-2         | `eip155:5042`                    | `eip155:5042002`                      |
| RPC URL        | `https://rpc.mainnet.arc.io`     | `https://rpc.testnet.arc.io`          |
| Block explorer | `https://explorer.arc.io`        | `https://testnet.explorer.arc.io`     |
| Gas token      | USDC (6 dec ERC-20 view)         | USDC                                  |
| Finality       | Deterministic, < 1 s             | Deterministic, < 1 s                  |
| Fee model      | EIP-1559, min 20 gwei ≈ $0.01   |                                       |
| Launched       | 2026-09-16                       |                                       |

---

## 2. ERC-8004 on Arc — What This Means

### `giveFeedback` on Arc

The `giveFeedback` call is identical to any other mainnet. Gas is paid in USDC (~$0.01).
The transaction lands in < 1 second.

```typescript
import { CHAIN_IDS, addressesForChain } from "@mnemonik-xyz/sdk/erc8004/registry";

// addressesForChain(5042) → MAINNET_ADDRESSES (5042 is not in TESTNET_IDS)
const { reputationRegistry } = addressesForChain(CHAIN_IDS.arcMainnet);
// → "0x8004BAa17C55a88189AE136b182e5fdA19dE9b63"
```

`agentRegistry` field in `MNEMONIC_FEEDBACK_V1` documents targeting Arc:
```
"agentRegistry": "eip155:5042:0x8004BAa17C55a88189AE136b182e5fdA19dE9b63"
```

### USDC gas funding

Each `giveFeedback` tx costs ~$0.01 in USDC. Agents must hold a small USDC balance on Arc.
Bridge via CCTP (domain 26) from any supported chain — the `CrossChainTokenService` at
`0x431871229103b780868f8C6BB820cd16ECf942BC` handles the relay.

---

## 3. Why Arc First

| Factor | Arc | Ethereum Mainnet |
|--------|-----|-----------------|
| Gas cost per `giveFeedback` | ~$0.01 USDC | ~$2–15 ETH gas |
| Gas token | USDC (familiar to agents) | ETH (requires ETH holding) |
| Finality | < 1 s | ~12 s (1 block) |
| USDC native | Yes (system contract) | Bridged ERC-20 |
| ERC-8004 deployed | ✓ confirmed | ✓ |
| Target user | AI agents, automated workflows | — |

Arc's USDC-native gas model is the clearest fit for Mnemonik: agents already hold USDC
for payments, so they need no separate gas token.

---

## 4. Code Changes Made (this PR)

### `packages/sdk/src/erc8004/registry.ts`

- Added `arcMainnet: 5042` and `arcTestnet: 5042002` to `CHAIN_IDS`
- Added `5042002` to the `TESTNET_IDS` set in `addressesForChain()` so Arc testnet
  correctly returns testnet registry addresses

---

## 5. Open Items

| # | Item | Status |
|---|------|--------|
| 1 | Add `arcMainnet` / `arcTestnet` to `CHAIN_IDS` in `registry.ts` | ✅ done (this PR) |
| 2 | Add Arc chain entry to the `agentRegistry` CAIP-2 format examples in `docs/spec/erc8004-feedback-v1.md` | TODO |
| 3 | Verify ERC-8004 test vectors against Arc testnet RPC | TODO |
| 4 | Agent gas-funding guide: how to bridge USDC from Ethereum to Arc via CCTP | TODO |
| 5 | CLI: add `--chain arc` alias to `mnemonic erc8004 feedback` | TODO |

---

## Sources

- [Arc Contract Addresses — docs.arc.io](https://docs.arc.io/arc/references/contract-addresses)
- [Arc Mainnet Chain ID & RPC](https://trustswap.com/arc/mainnet-live)
- [ERC-8004 on Arc mainnet (UltravioletaDAO)](https://github.com/UltravioletaDAO/uvd-x402-sdk-typescript/pull/35)
- [qntx/erc8004 — canonical registry addresses](https://github.com/qntx/erc8004)
