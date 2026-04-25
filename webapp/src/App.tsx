import { useState } from "react";
import LandingPage from "./components/LandingPage";

type View = "landing" | "chat";

function App() {
  const [view, setView] = useState<View>("landing");

  if (view === "chat") {
    return (
      <main className="flex min-h-screen flex-col items-center justify-center px-4">
        <p className="text-text-muted">Chat view — not yet implemented.</p>
        <button
          type="button"
          onClick={() => setView("landing")}
          className="mt-4 text-sm text-accent-primary underline underline-offset-4 hover:opacity-90"
        >
          Back to landing
        </button>
      </main>
    );
  }

  return <LandingPage onStartChat={() => setView("chat")} />;
}

export default App;
