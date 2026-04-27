import { Link } from "react-router-dom";
import IdentityPanel from "../components/IdentityPanel";
import InstallButtons from "../components/InstallButtons";

/**
 * Install hub (`/install`). Two columns on desktop, stacked on mobile:
 *   - Install buttons (Cursor / VS Code / Claude.ai).
 *   - Identity panel (Generate / Import / Export keypair).
 */
export default function Install() {
  return (
    <main className="min-h-screen px-4 py-10">
      <div className="mx-auto max-w-4xl space-y-8">
        <header className="space-y-2">
          <Link
            to="/"
            className="text-sm text-text-muted transition-colors hover:text-text-primary"
          >
            ← Back
          </Link>
          <h1 className="text-3xl font-bold tracking-tight text-text-primary">
            Install
          </h1>
          <p className="text-sm text-text-muted">
            Connect Mnemonic to your AI tool, then create or import the Ed25519
            keypair that will sign your memories.
          </p>
        </header>

        <div className="grid gap-8 md:grid-cols-2">
          <InstallButtons />
          <IdentityPanel />
        </div>
      </div>
    </main>
  );
}
