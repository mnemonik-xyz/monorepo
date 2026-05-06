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
  discord: "https://discord.gg/ws6wruJj",
  // Telegram group — placeholder; swap in the real invite link when issued.
  telegram: "https://t.me/mnemonik_xyz",
  issues: "https://github.com/mnemonik-xyz/monorepo/issues",
} as const;

export type ExternalLinkKey = keyof typeof EXTERNAL_LINKS;
