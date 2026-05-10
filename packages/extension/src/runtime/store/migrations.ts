// IndexedDB schema declaration + versioned migrations.
//
// One database (`mnemonik`), three object stores. Versioned: every
// migration step is gated by `oldVersion < N` so re-runs are idempotent
// across reinstalls.
//
// Schema lock: bumping `SCHEMA_VERSION` MUST be paired with a migration
// branch below. Never mutate an existing branch — only append (a user
// upgrading from v1 → v3 must replay v1→v2 then v2→v3 in order).

import type { DBSchema, IDBPDatabase, IDBPTransaction } from "idb";
import type { AttestationRow, LineageEdge, PendingUpload } from "./types.js";

export const DB_NAME = "mnemonik";
export const SCHEMA_VERSION = 1;

export interface MnemonikDb extends DBSchema {
  attestations: {
    key: string;
    value: AttestationRow;
    indexes: {
      by_signer: string;
      by_owner: string;
      by_created_at: string;
      by_tags: string;
    };
  };
  lineage_edges: {
    key: number;
    value: LineageEdge;
    indexes: {
      by_child: string;
      by_parent: string;
    };
  };
  pending_uploads: {
    key: string;
    value: PendingUpload;
    indexes: {
      by_enqueued_at: string;
    };
  };
}

/**
 * Apply the schema delta from `oldVersion` to `SCHEMA_VERSION`. Called
 * from `openDB`'s `upgrade` callback; runs inside the
 * `versionchange` transaction.
 */
export function applyMigrations(
  db: IDBPDatabase<MnemonikDb>,
  oldVersion: number,
  _tx: IDBPTransaction<MnemonikDb, ("attestations" | "lineage_edges" | "pending_uploads")[], "versionchange">,
): void {
  if (oldVersion < 1) {
    const attestations = db.createObjectStore("attestations", {
      keyPath: "attestation_id",
    });
    attestations.createIndex("by_signer", "signer_pubkey");
    attestations.createIndex("by_owner", "owner_pubkey");
    attestations.createIndex("by_created_at", "created_at");
    attestations.createIndex("by_tags", "tags", { multiEntry: true });

    const lineage = db.createObjectStore("lineage_edges", {
      keyPath: "id",
      autoIncrement: true,
    });
    lineage.createIndex("by_child", "child_id");
    lineage.createIndex("by_parent", "parent_id");

    const pending = db.createObjectStore("pending_uploads", {
      keyPath: "attestation_id",
    });
    pending.createIndex("by_enqueued_at", "enqueued_at");
  }
  // Future versions append here:
  // if (oldVersion < 2) { ... }
}
