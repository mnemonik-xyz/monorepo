import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Popup } from "./Popup.tsx";

const container = document.getElementById("root");
if (!container) {
  throw new Error("popup root missing");
}
createRoot(container).render(
  <StrictMode>
    <Popup />
  </StrictMode>,
);
