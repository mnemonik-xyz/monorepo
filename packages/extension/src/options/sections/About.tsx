// About section. Static metadata: extension version (read from
// `chrome.runtime.getManifest` in production via the runtime facade)
// and links to the privacy policy + GitHub repo. Build-hash is wired
// when the build system bakes one in (T20 packaging task).

import { useEffect, useState, type JSX, type ReactNode } from "react";
import { getOptionsRuntime } from "../runtime.js";

const PRIVACY_URL = "https://mnemonik.xyz/privacy";
const REPO_URL = "https://github.com/mnemonik-xyz/monorepo";

export function About(): JSX.Element {
  const [meta, setMeta] = useState<{ version: string; buildHash?: string }>({
    version: "0.0.0",
  });

  useEffect(() => {
    setMeta(getOptionsRuntime().about);
  }, []);

  return (
    <div className="flex flex-col gap-4">
      <header>
        <h2 className="text-sm font-mono uppercase tracking-wide text-accent-primary">
          About
        </h2>
      </header>

      <div className="border border-white/10 rounded p-3 flex flex-col gap-2 text-xs">
        <Row label="Version" value={meta.version} />
        {meta.buildHash ? <Row label="Build" value={meta.buildHash} /> : null}
        <Row
          label="Privacy"
          value={
            <a
              href={PRIVACY_URL}
              target="_blank"
              rel="noreferrer noopener"
              className="text-accent-primary hover:underline"
            >
              {PRIVACY_URL}
            </a>
          }
        />
        <Row
          label="Source"
          value={
            <a
              href={REPO_URL}
              target="_blank"
              rel="noreferrer noopener"
              className="text-accent-primary hover:underline"
            >
              {REPO_URL}
            </a>
          }
        />
      </div>
    </div>
  );
}

function Row({
  label,
  value,
}: {
  label: string;
  value: ReactNode;
}): JSX.Element {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-[10px] uppercase tracking-wide text-text-muted font-mono">
        {label}
      </span>
      <span className="font-mono break-all text-right">{value}</span>
    </div>
  );
}
