// Shadow-DOM-scoped CSS for the FAB and Recall overlay.
//
// Returned as a plain string so callers can adopt it via either
// `<style>` injection (broadest compatibility, including JSDOM) or
// adoptedStyleSheets in MV3 worlds where CSSStyleSheet is constructable.
// Tokens mirror the webapp:
//   - background: #0A0F1E (deep space)
//   - accent:     #00D4B4 (teal)
//   - text muted: #94A3B8
//   - mono font:  ui-monospace, Menlo, Monaco, Consolas
//
// IMPORTANT: every selector here is scoped under `:host` (the shadow
// root boundary). The host page cannot reach in via document selectors
// because the shadow root is `closed` and the host element resets all
// inherited styles via `all: initial` (set in fab.ts / recall-overlay.ts).

export const SHADOW_STYLES: string = `
:host {
  all: initial;
  font-family: ui-monospace, "SF Mono", Menlo, Monaco, Consolas, monospace;
  color: #E2E8F0;
}

/* ── FAB ─────────────────────────────────────────────────────────── */

.fab-root {
  position: fixed;
  z-index: 2147483646;
  display: flex;
  flex-direction: column-reverse;
  align-items: flex-end;
  gap: 8px;
}

.fab-button {
  width: 56px;
  height: 56px;
  border-radius: 50%;
  border: 1px solid rgba(0, 212, 180, 0.5);
  background: #0A0F1E;
  color: #00D4B4;
  cursor: grab;
  display: flex;
  align-items: center;
  justify-content: center;
  font-family: inherit;
  font-size: 18px;
  font-weight: 700;
  letter-spacing: 0.05em;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4),
              0 0 0 1px rgba(0, 212, 180, 0.2);
  transition: transform 120ms ease, box-shadow 120ms ease;
  user-select: none;
}

.fab-button:hover {
  transform: scale(1.05);
  box-shadow: 0 6px 20px rgba(0, 212, 180, 0.3),
              0 0 0 2px rgba(0, 212, 180, 0.4);
}

.fab-button:active,
.fab-button.dragging {
  cursor: grabbing;
  transform: scale(0.97);
}

.fab-menu {
  display: flex;
  flex-direction: column;
  gap: 4px;
  background: #0A0F1E;
  border: 1px solid rgba(0, 212, 180, 0.3);
  border-radius: 8px;
  padding: 6px;
  min-width: 180px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
}

.fab-menu[hidden] {
  display: none;
}

.fab-menu-item {
  background: transparent;
  border: none;
  color: #E2E8F0;
  font-family: inherit;
  font-size: 12px;
  text-align: left;
  padding: 8px 10px;
  border-radius: 4px;
  cursor: pointer;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.fab-menu-item:hover,
.fab-menu-item:focus-visible {
  background: rgba(0, 212, 180, 0.15);
  color: #00D4B4;
  outline: none;
}

/* ── Recall overlay ──────────────────────────────────────────────── */

.overlay-backdrop {
  position: fixed;
  inset: 0;
  z-index: 2147483647;
  background: rgba(10, 15, 30, 0.78);
  backdrop-filter: blur(2px);
  display: flex;
  align-items: center;
  justify-content: center;
  pointer-events: auto;
}

.overlay-modal {
  width: min(640px, 92vw);
  max-height: 80vh;
  background: #0A0F1E;
  border: 1px solid rgba(0, 212, 180, 0.4);
  border-radius: 12px;
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  box-shadow: 0 24px 64px rgba(0, 0, 0, 0.6);
}

.overlay-input {
  width: 100%;
  box-sizing: border-box;
  background: rgba(0, 0, 0, 0.4);
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 6px;
  padding: 10px 12px;
  color: #E2E8F0;
  font-family: inherit;
  font-size: 13px;
}

.overlay-input:focus {
  outline: none;
  border-color: rgba(0, 212, 180, 0.6);
}

.overlay-status {
  font-size: 11px;
  color: #94A3B8;
  font-style: italic;
}

.overlay-status.error {
  color: #F87171;
  font-style: normal;
}

.overlay-results {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  overflow-y: auto;
  max-height: 60vh;
}

.overlay-result {
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 6px;
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.overlay-result.focused {
  border-color: rgba(0, 212, 180, 0.6);
  background: rgba(0, 212, 180, 0.05);
}

.overlay-result-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.overlay-result-score {
  color: #00D4B4;
}

.overlay-result-pill {
  border: 1px solid rgba(255, 255, 255, 0.15);
  border-radius: 3px;
  padding: 1px 6px;
  color: #94A3B8;
}

.overlay-result-snippet {
  font-size: 12px;
  color: #E2E8F0;
  line-height: 1.4;
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.overlay-result-actions {
  display: flex;
  gap: 6px;
}

.overlay-action {
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  font-family: inherit;
  background: transparent;
  color: #94A3B8;
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 4px;
  padding: 3px 8px;
  cursor: pointer;
  text-decoration: none;
}

.overlay-action:hover:not([disabled]),
.overlay-action:focus-visible:not([disabled]) {
  color: #00D4B4;
  border-color: rgba(0, 212, 180, 0.5);
  outline: none;
}

.overlay-action[disabled] {
  opacity: 0.4;
  cursor: not-allowed;
}
`;
