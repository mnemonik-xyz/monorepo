export function Popup() {
  return (
    <main
      style={{
        width: 320,
        minHeight: 200,
        padding: 16,
        background: "#0A0F1E",
        color: "#E5E7EB",
        fontFamily:
          "ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, sans-serif",
      }}
    >
      <header style={{ marginBottom: 12 }}>
        <h1 style={{ margin: 0, fontSize: 14, color: "#00D4B4" }}>Mnemonik</h1>
        <p style={{ margin: "4px 0 0", fontSize: 11, opacity: 0.6 }}>
          v0.1.0 · scaffolding
        </p>
      </header>
      <section style={{ fontSize: 12, lineHeight: 1.5 }}>
        Hello from the Mnemonik popup. Real capture / recall UI lands in T07.
      </section>
    </main>
  );
}
