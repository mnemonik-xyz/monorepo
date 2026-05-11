// Passphrase strength meter backed by @zxcvbn-ts/core. Renders a 5-
// bucket bar + score label and exposes acceptance via the `onChange`
// callback so the parent can disable its submit button.
//
// Acceptance rule (matches user-spec Scenario 1 + the audit checklist):
//   length >= 12 AND zxcvbn score >= 3
//
// Dictionary loading: zxcvbn-ts ships its dictionaries lazily through
// `loadOptions`. We initialise once per module load — the cost is
// amortised across rotate + export forms. The component itself is meant
// to be lazy-imported via React.lazy from the consuming sections so the
// options-page first-paint bundle doesn't carry the ~50KB gzip cost.

import { useEffect, useMemo, type JSX } from "react";
import { zxcvbn, zxcvbnOptions } from "@zxcvbn-ts/core";
import {
  translations as enTranslations,
  dictionary as enDictionary,
} from "@zxcvbn-ts/language-en";
import {
  adjacencyGraphs as commonAdjacencyGraphs,
  dictionary as commonDictionary,
} from "@zxcvbn-ts/language-common";

let configured = false;
function ensureConfigured(): void {
  if (configured) return;
  zxcvbnOptions.setOptions({
    translations: enTranslations,
    dictionary: { ...commonDictionary, ...enDictionary },
    graphs: commonAdjacencyGraphs,
  });
  configured = true;
}

export const MIN_PASSPHRASE_LENGTH = 12;
export const MIN_PASSPHRASE_SCORE = 3;

const SCORE_LABELS: Record<0 | 1 | 2 | 3 | 4, string> = {
  0: "Very weak",
  1: "Weak",
  2: "Fair",
  3: "Strong",
  4: "Very strong",
};

const SCORE_COLORS: Record<0 | 1 | 2 | 3 | 4, string> = {
  0: "bg-error",
  1: "bg-error",
  2: "bg-warning",
  3: "bg-accent-primary",
  4: "bg-accent-primary",
};

export interface PassphraseStrengthProps {
  /** The passphrase to evaluate. Empty string renders a neutral bar. */
  value: string;
  /** Called whenever the acceptance verdict changes — parent disables
   *  its submit button when this is `false`. */
  onAcceptableChange?: (acceptable: boolean) => void;
}

function PassphraseStrength({
  value,
  onAcceptableChange,
}: PassphraseStrengthProps): JSX.Element {
  ensureConfigured();
  const trimmed = value.trim();
  const score = useMemo<0 | 1 | 2 | 3 | 4>(() => {
    if (!trimmed) return 0;
    const result = zxcvbn(trimmed);
    return Math.min(4, Math.max(0, result.score)) as 0 | 1 | 2 | 3 | 4;
  }, [trimmed]);
  const longEnough = trimmed.length >= MIN_PASSPHRASE_LENGTH;
  const acceptable = longEnough && score >= MIN_PASSPHRASE_SCORE;

  useEffect(() => {
    onAcceptableChange?.(acceptable);
  }, [acceptable, onAcceptableChange]);

  return (
    <div
      className="flex flex-col gap-1"
      data-testid="passphrase-strength"
      aria-live="polite"
    >
      <div
        className="flex gap-0.5"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={4}
        aria-valuenow={trimmed ? score : 0}
        aria-label="Passphrase strength"
      >
        {[0, 1, 2, 3, 4].map((bucket) => {
          const filled = trimmed.length > 0 && bucket <= score;
          return (
            <div
              key={bucket}
              className={`h-1 flex-1 rounded ${
                filled ? SCORE_COLORS[score] : "bg-white/10"
              }`}
            />
          );
        })}
      </div>
      <div className="flex items-center justify-between text-[10px] font-mono text-text-muted">
        <span data-testid="strength-label">
          {trimmed ? SCORE_LABELS[score] : "Enter a passphrase"}
        </span>
        <span>
          {trimmed.length}/{MIN_PASSPHRASE_LENGTH}+ chars
          {!acceptable && trimmed.length > 0 ? " · need score 3+" : ""}
        </span>
      </div>
    </div>
  );
}

export default PassphraseStrength;

/** Pure helper for tests + parent acceptance gating without a render. */
export function isPassphraseAcceptable(value: string): boolean {
  ensureConfigured();
  const trimmed = value.trim();
  if (trimmed.length < MIN_PASSPHRASE_LENGTH) return false;
  return zxcvbn(trimmed).score >= MIN_PASSPHRASE_SCORE;
}
