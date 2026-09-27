// ERC-8004 contract addresses and ABI — pinned from verified on-chain sources.
//
// Source: qntx/erc8004 (https://github.com/qntx/erc8004), networks.rs
// Fetched: 2026-09-27
// Contracts are deployed via CREATE2 — same addresses across all mainnets,
// same addresses across all testnets.
//
// Mainnet Identity Registry:   0x8004A169FB4a3325136EB29fA0ceB6D2e539a432
// Mainnet Reputation Registry: 0x8004BAa17C55a88189AE136b182e5fdA19dE9b63
// Testnet Identity Registry:   0x8004A818BFB912233c491871b3d84c89A494BD9e
// Testnet Reputation Registry: 0x8004B663056A597Dffe9eCcC1965A193B7388713
//
// Source: EIPs-CodeLab/ERC-8004 (https://github.com/EIPs-CodeLab/ERC-8004), src/ReputationRegistry.sol
// Fetched: 2026-09-27
// giveFeedback selector: keccak256("giveFeedback(uint256,int128,uint8,string,string,string,string,bytes32)")[0:4]
// Verified: cast keccak "giveFeedback(uint256,int128,uint8,string,string,string,string,bytes32)" | head -c 10

export interface ChainAddresses {
  readonly identityRegistry: `0x${string}`;
  readonly reputationRegistry: `0x${string}`;
  readonly chainId: number;
}

// Mainnet chains (Ethereum, Base, Polygon, Arbitrum, and others — all share
// the same CREATE2 addresses).
export const MAINNET_ADDRESSES = {
  identityRegistry: "0x8004A169FB4a3325136EB29fA0ceB6D2e539a432",
  reputationRegistry: "0x8004BAa17C55a88189AE136b182e5fdA19dE9b63",
} as const;

// Testnet chains (Sepolia, Base Sepolia, etc. — all share the same CREATE2
// addresses).
export const TESTNET_ADDRESSES = {
  identityRegistry: "0x8004A818BFB912233c491871b3d84c89A494BD9e",
  reputationRegistry: "0x8004B663056A597Dffe9eCcC1965A193B7388713",
} as const;

// Chain IDs for commonly used networks, overridable by flag.
// Arc is Circle's native USDC L1 (launched 2026-09-16). ERC-8004 contracts
// are confirmed deployed via CREATE2 at the standard mainnet addresses on Arc.
export const CHAIN_IDS = {
  ethereumMainnet: 1,
  baseMainnet: 8453,
  ethereumSepolia: 11155111,
  baseSepolia: 84532,
  arcMainnet: 5042,
  arcTestnet: 5042002,
} as const;

// Returns registry addresses for a given chain ID.
// Any chain not listed here is assumed mainnet-class (same CREATE2 result).
// The caller can always override via the `registryOverride` option.
export function addressesForChain(chainId: number): typeof MAINNET_ADDRESSES | typeof TESTNET_ADDRESSES {
  const TESTNET_IDS = new Set([11155111, 84532, 80002, 421614, 44787, 534351, 97, 43113, 59141, 5003, 6342, 11155420, 5042002]);
  return TESTNET_IDS.has(chainId) ? TESTNET_ADDRESSES : MAINNET_ADDRESSES;
}

// Pinned ABI for ReputationRegistry.giveFeedback.
//
// Source: EIPs-CodeLab/ERC-8004 src/ReputationRegistry.sol, verified 2026-09-27.
// Full signature: giveFeedback(uint256,int128,uint8,string,string,string,string,bytes32)
//
// IMPORTANT: value is int128 (signed), NOT uint256.
// IMPORTANT: tag1/tag2 are string, NOT bytes32.
// IMPORTANT: endpoint is an extra string param between tag2 and feedbackURI.
export const GIVE_FEEDBACK_SIG =
  "giveFeedback(uint256,int128,uint8,string,string,string,string,bytes32)" as const;

// 4-byte selector — keccak256(GIVE_FEEDBACK_SIG)[0:4]
// Verification: cast keccak "giveFeedback(uint256,int128,uint8,string,string,string,string,bytes32)"
// Expected output starts with this value.
// Asserted in erc8004.abi.test.ts.
export const GIVE_FEEDBACK_SELECTOR = "0xd5d1e4af" as const;

// Self-promotion guard: these require() checks mirror what the contract enforces.
// The client runs them pre-flight so a doomed tx is never sent.
//   1. msg.sender != ownerOf(agentId)
//   2. !isApprovedForAll(owner, msg.sender)
//   3. getApproved(agentId) != msg.sender
export const SELF_PROMOTION_REQUIRE_LIST = [
  "ownerOf",
  "isApprovedForAll",
  "getApproved",
] as const;

// Tag convention: tags are passed as plain strings to the contract (string
// calldata), so no bytes32 conversion is needed. The contract's getSummary
// internally uses keccak256 for tag matching, but the on-chain storage and
// events keep the raw string — a tag of "quality" is passed as "quality".
export const TAG_CONVENTION = "string-passthrough" as const;

// feedbackURI is emitted in the NewFeedback event and stored in the Feedback
// struct only implicitly (via the event log). The contract does NOT persist
// feedbackURI in its storage mapping — only feedbackHash is stored in the
// Feedback struct. Consequence: a data: URI is technically valid but large
// (costs calldata gas); the hash is always on-chain regardless of URI liveness.
export const FEEDBACK_URI_STORAGE = "event-only" as const;
