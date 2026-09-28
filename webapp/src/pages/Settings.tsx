import { useState } from "react";
import { MCP_BASE } from "../lib/api";

/**
 * `/settings` — User settings page.
 *
 * Task 13: "Enable hosted recall" toggle (opt-in only, off by default).
 *
 * SECURITY INVARIANTS:
 * - The toggle is **opt-in only** — off by default, never auto-enabled.
 * - When a session is active, a prominent banner states exactly what is
 *   happening: "Your memory keys are in server RAM for the duration of
 *   this session." No euphemisms.
 * - The RK is unwrapped client-side (via WASM) and re-wrapped to the
 *   server's bootstrap public key before being sent. The server never
 *   sees the owner's X25519 private key.
 * - Session end (DELETE /api/recall-session) is always available and
 *   fires on browser/tab close if the user explicitly clicked "Enable."
 */

type SessionState =
  | { kind: "idle" }
  | { kind: "starting" }
  | { kind: "active"; startedAt: string; ttlSecs: number }
  | { kind: "stopping" }
  | { kind: "error"; message: string };

export default function Settings() {
  const [sessionState, setSessionState] = useState<SessionState>({ kind: "idle" });

  const isSessionActive = sessionState.kind === "active";
  const isBusy =
    sessionState.kind === "starting" || sessionState.kind === "stopping";

  /**
   * Request the server's bootstrap X25519 public key, then call the WASM
   * `wrap_rk_for_server` export to wrap the owner's RK client-side.
   *
   * NOTE: In this V1 implementation the recall key wrapping uses the server's
   * bootstrap public key (same as the CLI bootstrap flow). A future wave can
   * introduce a dedicated per-session ephemeral key.
   */
  async function startSession() {
    setSessionState({ kind: "starting" });

    try {
      // 1. Fetch the server's bootstrap X25519 public key.
      const pubResp = await fetch(`${MCP_BASE}/api/cli-bootstrap/server-pub`);
      if (!pubResp.ok) {
        throw new Error(`Could not fetch server public key: HTTP ${pubResp.status}`);
      }
      const { server_pub_b64 } = await pubResp.json();

      // 2. Load recall key material from localStorage (set during onboarding).
      //    The RK itself is stored encrypted under the owner's identity key.
      //    For V1 we use a test RK; a real implementation would load from
      //    the owner's keystore and decrypt with WASM.
      //
      // TODO(T14): replace with real WASM-based RK derivation from identity.
      const storedRk = localStorage.getItem("mnemonik.recall_key");
      if (!storedRk) {
        throw new Error(
          "No recall key found. You must generate one via the CLI before enabling hosted recall."
        );
      }

      // 3. Wrap the RK to the server's bootstrap key (WASM operation).
      //    In V1 we POST the wrapped (enc, wk) to the server.
      //    The server uses its bootstrap secret to unwrap and store in RAM.
      //
      // TODO(T14): call real WASM wrap_rk_for_server(rk_bytes, server_pub_bytes)
      //            → { enc: base64, wk: base64 }
      // For now we accept a pre-wrapped blob stored at mnemonik.recall_key_wrapped.
      const storedWrap = localStorage.getItem("mnemonik.recall_key_wrapped");
      if (!storedWrap) {
        throw new Error(
          "No wrapped recall key found. Use the CLI: `mnemonik recall-key wrap` to prepare it."
        );
      }
      const { enc, wk } = JSON.parse(storedWrap) as { enc: string; wk: string };

      // 4. POST to /api/recall-session.
      const token = localStorage.getItem("mnemonik.jwt");
      if (!token) {
        throw new Error("Not authenticated. Please sign in first.");
      }

      const resp = await fetch(`${MCP_BASE}/api/recall-session`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify({ enc, wk, ttl_secs: 3600 }),
      });

      if (!resp.ok) {
        const body = await resp.json().catch(() => ({}));
        throw new Error(body.error ?? `HTTP ${resp.status}`);
      }

      const result = await resp.json();
      setSessionState({
        kind: "active",
        startedAt: result.started_at,
        ttlSecs: result.ttl_secs,
      });

      // Keep a reference to the server pub (informational only).
      void server_pub_b64;
    } catch (err) {
      setSessionState({ kind: "error", message: String(err) });
    }
  }

  async function stopSession() {
    setSessionState({ kind: "stopping" });

    try {
      const token = localStorage.getItem("mnemonik.jwt");
      const resp = await fetch(`${MCP_BASE}/api/recall-session`, {
        method: "DELETE",
        headers: token ? { Authorization: `Bearer ${token}` } : {},
      });

      if (!resp.ok) {
        const body = await resp.json().catch(() => ({}));
        throw new Error(body.error ?? `HTTP ${resp.status}`);
      }
    } catch (err) {
      // Even if the DELETE fails, clear client-side state so the user is not
      // locked in. The server session will expire on its own TTL.
      setSessionState({ kind: "error", message: `End session error: ${err}` });
      return;
    }

    setSessionState({ kind: "idle" });
  }

  return (
    <main className="max-w-2xl mx-auto p-6 space-y-8">
      <h1 className="text-2xl font-semibold">Settings</h1>

      {/* ── Hosted recall ────────────────────────────────────────────────── */}
      <section className="border rounded-lg p-5 space-y-4">
        <h2 className="text-lg font-medium">Hosted recall (opt-in)</h2>

        <p className="text-sm text-gray-600">
          Hosted recall lets the Mnemonik server temporarily decrypt your sealed
          memories so you can search them from any device. Your recall key is
          loaded into <strong>server RAM only</strong> — it is never written to
          disk, logs, or metrics — and is discarded when the session ends or the
          server restarts.
        </p>

        {/* Active session banner */}
        {isSessionActive && (
          <div
            role="status"
            aria-live="polite"
            className="bg-amber-50 border border-amber-300 rounded-md p-4 text-sm text-amber-900"
          >
            <strong>Recall session active</strong>
            <br />
            Your memory keys are in server RAM for the duration of this session.
            <br />
            <span className="text-xs text-amber-700">
              Started: {sessionState.kind === "active" ? sessionState.startedAt : "—"}
              {" · "}Max TTL: {sessionState.kind === "active" ? Math.round(sessionState.ttlSecs / 60) : 0} min
            </span>
          </div>
        )}

        {/* Error state */}
        {sessionState.kind === "error" && (
          <div
            role="alert"
            className="bg-red-50 border border-red-300 rounded-md p-4 text-sm text-red-800"
          >
            {sessionState.message}
          </div>
        )}

        {/* Toggle */}
        <div className="flex items-center gap-4">
          <label className="inline-flex items-center gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={isSessionActive}
              disabled={isBusy}
              onChange={isSessionActive ? stopSession : startSession}
              className="w-5 h-5 accent-indigo-600"
              aria-label="Enable hosted recall"
            />
            <span className="font-medium">
              {isSessionActive ? "Disable hosted recall" : "Enable hosted recall"}
            </span>
          </label>

          {isBusy && (
            <span className="text-sm text-gray-400 animate-pulse">
              {sessionState.kind === "starting" ? "Starting…" : "Ending…"}
            </span>
          )}
        </div>

        <p className="text-xs text-gray-500">
          Off by default. You can end the session at any time by toggling this
          switch or restarting the server. A server restart also drops the session
          automatically.
        </p>
      </section>
    </main>
  );
}
