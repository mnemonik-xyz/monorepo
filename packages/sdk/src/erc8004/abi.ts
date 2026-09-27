// Minimal ABI encoder for ReputationRegistry.giveFeedback calldata.
//
// Encodes: uint256, int128, uint8, bytes32 (static) and string (dynamic).
// ABI spec: https://docs.soliditylang.org/en/latest/abi-spec.html
//
// giveFeedback(uint256,int128,uint8,string,string,string,string,bytes32)
// selector: 0x3c036a7e
// Verified: keccak256("giveFeedback(uint256,int128,uint8,string,string,string,string,bytes32)")[0:4]

import {
  encodeBytes32,
  encodeInt128,
  encodeUint256,
  encodeUint8,
  hexToBytes,
} from "./evm.js";
import { GIVE_FEEDBACK_SELECTOR } from "./registry.js";

export interface GiveFeedbackArgs {
  agentId: bigint | number;
  value: bigint | number;
  valueDecimals: number;
  tag1: string;
  tag2: string;
  endpoint: string;
  feedbackURI: string;
  feedbackHash: string | Uint8Array;
}

// Returns the full calldata: 4-byte selector + ABI-encoded arguments.
export function encodeGiveFeedback(args: GiveFeedbackArgs): Uint8Array {
  const selector = hexToBytes(GIVE_FEEDBACK_SELECTOR);
  const body = abiEncode(args);
  const out = new Uint8Array(4 + body.length);
  out.set(selector, 0);
  out.set(body, 4);
  return out;
}

// ABI-encodes the 8 arguments without the selector.
//
// Layout: head (8 × 32 = 256 bytes) followed by tail (dynamic data).
// Static slots hold the value directly.
// Dynamic slots hold the byte offset from the start of the encoding
// to the corresponding tail entry.
function abiEncode(args: GiveFeedbackArgs): Uint8Array {
  const enc = new TextEncoder();

  // Static slots
  const s0 = encodeUint256(args.agentId);
  const s1 = encodeInt128(args.value);
  const s2 = encodeUint8(args.valueDecimals);
  const s7 = encodeBytes32(args.feedbackHash);

  // Dynamic payloads (UTF-8 encoded)
  const d3 = enc.encode(args.tag1);
  const d4 = enc.encode(args.tag2);
  const d5 = enc.encode(args.endpoint);
  const d6 = enc.encode(args.feedbackURI);

  const HEAD = 256; // 8 × 32

  // Compute tail offsets (measured from byte 0 of the encoding, i.e. after selector)
  const off3 = HEAD;
  const off4 = off3 + dynamicSlotSize(d3);
  const off5 = off4 + dynamicSlotSize(d4);
  const off6 = off5 + dynamicSlotSize(d5);

  // Build head
  const head = new Uint8Array(HEAD);
  let p = 0;
  head.set(s0, p); p += 32;
  head.set(s1, p); p += 32;
  head.set(s2, p); p += 32;
  head.set(encodeUint256(off3), p); p += 32;
  head.set(encodeUint256(off4), p); p += 32;
  head.set(encodeUint256(off5), p); p += 32;
  head.set(encodeUint256(off6), p); p += 32;
  head.set(s7, p);

  // Build tail
  const tail3 = encodeDynamic(d3);
  const tail4 = encodeDynamic(d4);
  const tail5 = encodeDynamic(d5);
  const tail6 = encodeDynamic(d6);

  const totalTail = tail3.length + tail4.length + tail5.length + tail6.length;
  const out = new Uint8Array(HEAD + totalTail);
  out.set(head, 0);
  let tp = HEAD;
  out.set(tail3, tp); tp += tail3.length;
  out.set(tail4, tp); tp += tail4.length;
  out.set(tail5, tp); tp += tail5.length;
  out.set(tail6, tp);

  return out;
}

// 32-byte length prefix + data zero-padded to next 32-byte boundary.
function encodeDynamic(data: Uint8Array): Uint8Array {
  const padded = pad32(data.length);
  const out = new Uint8Array(32 + padded);
  out.set(encodeUint256(data.length), 0);
  out.set(data, 32);
  return out;
}

// Total bytes a dynamic slot occupies in the tail: 32 (length) + padded data.
function dynamicSlotSize(data: Uint8Array): number {
  return 32 + pad32(data.length);
}

// Round up to next multiple of 32.
function pad32(n: number): number {
  return Math.ceil(n / 32) * 32;
}
