/**
 * Single source of truth for external resource URLs surfaced in the UI.
 *
 * Update here once and every header/footer/button picks it up.
 */

export const EXTERNAL_LINKS = {
  github: "https://github.com/mnemonik-xyz/monorepo",
  whitepaper:
    "https://github.com/mnemonik-xyz/monorepo/blob/main/docs/WHITEPAPER.md",
  quickstart:
    "https://github.com/mnemonik-xyz/monorepo/blob/main/docs/quickstart.md",
  howItWorks:
    "https://github.com/mnemonik-xyz/monorepo/blob/main/docs/how-it-works.md",
  paper:
    "https://github.com/mnemonik-xyz/monorepo/blob/main/docs/research/paper.pdf",
  researchgate:
    "https://www.researchgate.net/publication/404381758_Sublinear_Verifiable_Recall_An_Inverted-File_Cascade_for_Compressed_Embedding_Retrieval_in_the_Mnemonic_Protocol",
  discord: "https://discord.gg/ws6wruJj",
  telegram: "https://t.me/mnemonikprotocol",
  telegramAnnouncements: "https://t.me/mnemonik_xyz_announcements",
  issues: "https://github.com/mnemonik-xyz/monorepo/issues",
} as const;

export type ExternalLinkKey = keyof typeof EXTERNAL_LINKS;

/**
 * Block-explorer URL builders for the on-chain anchors attached to an
 * attestation. `local:`-prefixed tx ids are synthetic (offline / on-node only)
 * and have no explorer page — callers should treat a `null` return as
 * "not anchored, render as plain text".
 */
export function solanaTxUrl(tx: string | null | undefined): string | null {
  if (!tx || tx.startsWith("local:")) return null;
  return `https://explorer.solana.com/tx/${encodeURIComponent(tx)}`;
}

export function arweaveTxUrl(tx: string | null | undefined): string | null {
  if (!tx || tx.startsWith("local:")) return null;
  return `https://viewblock.io/arweave/tx/${encodeURIComponent(tx)}`;
}
