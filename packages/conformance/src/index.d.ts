/** One A2A conformance vector entry. */
export interface A2aVector {
  name: string;
  a2a_json: string;
  jcs_canonical_hex: string;
  cbor_envelope_hex: string;
  cose_signed_hex: string;
  keypair_secret_hex: string;
}

export declare const vectors: A2aVector[];
export declare function loadVectors(): A2aVector[];
export declare function vectorsForKind(kind: "message" | "task" | "artifact"): A2aVector[];
