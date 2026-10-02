// Self-promotion guard for the ERC-8004 Reputation Registry.
//
// The registry forbids the agent's owner and their approved operators from
// rating it — any such tx is a guaranteed revert and wasted gas. This module
// checks those conditions client-side so a doomed tx is never sent.
//
// Three levels of checks:
//   1. Offline always: reject zero address, bad EIP-55 checksum.
//   2. Online (RPC URL available): eth_call ownerOf / isApprovedForAll /
//      getApproved. Emits a warning (not an error) when the RPC is
//      unreachable, so the caller can still build calldata.
//   3. Never blocks on: Sybil detection, collusion, fresh-wallet spam.
//      Those are explicitly out of scope and documented in the result.

import {
  bytesToHex,
  encodeUint256,
  hexToBytes,
  isValidChecksumAddress,
  keccak256Str,
} from "./evm.js";
import { addressesForChain, CHAIN_IDS } from "./registry.js";

export interface CheckSelfPromotionInput {
  agentId: string | bigint;
  clientAddress: string;
  chainId?: number;
  /** Identity Registry address override (resolves automatically if absent). */
  identityRegistryAddress?: string;
  /** RPC endpoint for eth_call checks. Omit to skip on-chain checks. */
  rpcUrl?: string;
}

export type SelfPromotionStatus =
  | "clean"      // passed all checks
  | "self"       // clientAddress == ownerOf(agentId)
  | "operator"   // isApprovedForAll(owner, clientAddress) == true
  | "approved"   // getApproved(agentId) == clientAddress
  | "skipped"    // no rpcUrl — only offline checks passed
  | "rpc-error"; // rpcUrl provided but eth_call failed

export interface SelfPromotionCheckResult {
  status: SelfPromotionStatus;
  warnings: string[];
}

// ── Well-known ERC-721 function selectors (keccak256 of sig, first 4 bytes)
// Asserted in erc8004.selfpromotion.test.ts against live computation.
const SEL_OWNER_OF = "0x6352211e";            // ownerOf(uint256)
const SEL_IS_APPROVED_FOR_ALL = "0xe985e9c5"; // isApprovedForAll(address,address)
const SEL_GET_APPROVED = "0x081812fc";         // getApproved(uint256)

export async function checkSelfPromotion(
  input: CheckSelfPromotionInput
): Promise<SelfPromotionCheckResult> {
  const warnings: string[] = [];

  // ── Offline checks ─────────────────────────────────────────────────────

  if (
    input.clientAddress === "0x0000000000000000000000000000000000000000" ||
    !isValidChecksumAddress(input.clientAddress)
  ) {
    return {
      status: "self",
      warnings: ["clientAddress is the zero address or has an invalid EIP-55 checksum"],
    };
  }

  // ── Resolve registry address ───────────────────────────────────────────

  const chainId = input.chainId ?? CHAIN_IDS.ethereumMainnet;
  const addrs = addressesForChain(chainId);
  const identityRegistry = input.identityRegistryAddress ?? addrs.identityRegistry;
  const agentId = BigInt(input.agentId);
  const clientLower = input.clientAddress.toLowerCase();

  // ── Online checks ──────────────────────────────────────────────────────

  if (!input.rpcUrl) {
    warnings.push(
      "self-promotion check skipped — no --rpc-url provided; a doomed tx will revert on chain"
    );
    return { status: "skipped", warnings };
  }

  try {
    // Check 1: ownerOf(agentId)
    const ownerResult = await ethCall(
      input.rpcUrl,
      identityRegistry,
      SEL_OWNER_OF + bytesToHex(encodeUint256(agentId))
    );
    if (ownerResult) {
      const owner = decodeAddress(ownerResult);
      if (!owner) throw new Error("invalid ownerOf response");
      if (owner && owner.toLowerCase() === clientLower) {
        return { status: "self", warnings };
      }

      // Check 2: isApprovedForAll(owner, clientAddress)
      const ownerBytes = hexToBytes(owner.slice(2));
      const clientBytes = hexToBytes(input.clientAddress.slice(2));
      const approvedAllData =
        SEL_IS_APPROVED_FOR_ALL +
        "000000000000000000000000" + bytesToHex(ownerBytes) +
        "000000000000000000000000" + bytesToHex(clientBytes);

      const approvedAllResult = await ethCall(
        input.rpcUrl,
        identityRegistry,
        approvedAllData
      );
      if (approvedAllResult && decodeBool(approvedAllResult)) {
        return { status: "operator", warnings };
      }
    }

    // Check 3: getApproved(agentId)
    const approvedResult = await ethCall(
      input.rpcUrl,
      identityRegistry,
      SEL_GET_APPROVED + bytesToHex(encodeUint256(agentId))
    );
    if (approvedResult) {
      const approved = decodeAddress(approvedResult);
      if (approved && approved.toLowerCase() === clientLower) {
        return { status: "approved", warnings };
      }
    }

    return { status: "clean", warnings };
  } catch (e) {
    warnings.push(
      `self-promotion RPC check failed: ${e instanceof Error ? e.message : String(e)}`
    );
    return { status: "rpc-error", warnings };
  }
}

// ── ABI helpers ────────────────────────────────────────────────────────────

async function ethCall(
  rpcUrl: string,
  to: string,
  data: string
): Promise<string | null> {
  const resp = await fetch(rpcUrl, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "eth_call",
      params: [{ to, data: data.startsWith("0x") ? data : `0x${data}` }, "latest"],
    }),
  });
  if (!resp.ok) throw new Error(`RPC HTTP ${resp.status}`);
  const json = (await resp.json()) as { result?: string; error?: { message: string } };
  if (json.error) throw new Error(`RPC error: ${json.error.message}`);
  return json.result ?? null;
}

// Decode a 32-byte ABI address result to EIP-55 string.
// Returns null if result is empty or zero address.
function decodeAddress(hex: string): string | null {
  const h = hex.startsWith("0x") ? hex.slice(2) : hex;
  if (h.length < 64) return null;
  const addr = "0x" + h.slice(24, 64);
  if (addr === "0x0000000000000000000000000000000000000000") return null;
  return addr;
}

// Decode a 32-byte ABI bool result.
function decodeBool(hex: string): boolean {
  const h = hex.startsWith("0x") ? hex.slice(2) : hex;
  if (h.length < 64) return false;
  return h[63] === "1";
}

// Exported so the test can verify selector values.
export function computeSelector(sig: string): string {
  const hash = keccak256Str(sig);
  return "0x" + bytesToHex(hash.slice(0, 4));
}

export { SEL_OWNER_OF, SEL_IS_APPROVED_FOR_ALL, SEL_GET_APPROVED };
