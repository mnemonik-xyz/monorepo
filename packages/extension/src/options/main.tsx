import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

function Options() {
  return (
    <div style={{ padding: 24, maxWidth: 720 }}>
      <h1 style={{ margin: 0, fontSize: 18, color: "#00D4B4" }}>Mnemonik — Settings</h1>
      <p style={{ marginTop: 12, fontSize: 13 }}>Storage mode, identity, and per-domain auto-capture toggles will live here.</p>
    </div>
  );
}

const container = document.getElementById("root");
if (!container) {
  throw new Error("options root missing");
}
createRoot(container).render(
  <StrictMode>
    <Options />
  </StrictMode>,
);
