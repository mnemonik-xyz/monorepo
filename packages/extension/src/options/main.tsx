// Options page entrypoint. Mounts <App /> into #root and registers the
// production OptionsRuntime once at boot. Tests bypass this file and
// call `setOptionsRuntime(stub)` directly in `beforeEach`.

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App.js";
import { setOptionsRuntime } from "./runtime.js";
import { createDefaultOptionsRuntime } from "./runtime-impl.js";
import "./styles.css";

setOptionsRuntime(createDefaultOptionsRuntime());

const container = document.getElementById("root");
if (!container) throw new Error("options root element missing");
createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
