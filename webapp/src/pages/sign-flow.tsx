import React, { useState } from "react";

const SignFlowPage: React.FC = () => {
  const [content, setContent] = useState("");
  const [result, setResult] = useState("");
  const [error, setError] = useState("");
  const [signing, setSigning] = useState(false);

  const handleSignMemory = async () => {
    if (!content.trim()) {
      setError("Content is required");
      return;
    }

    setSigning(true);
    setError("");
    setResult("");

    try {
      const response = await fetch("/mcp/tools/call", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${localStorage.getItem("jwt")}`,
        },
        body: JSON.stringify({
          tool: "mnemonic_sign_memory",
          arguments: { content },
        }),
      });

      if (!response.ok) {
        const err = await response.json().catch(() => ({}));
        throw new Error(err.message || `HTTP ${response.status}`);
      }

      const data = await response.json();
      
      if (data.status === "awaiting_signature") {
        // Poll for completion
        let attempts = 0;
        while (attempts < 30) {
          const statusRes = await fetch(`/api/pending/${data.correlation_id}`);
          const status = await statusRes.json();
          if (status.status === "completed") {
            setResult(`Success! arweave_tx:${status.arweaveTx} solana_tx:${status.solanaTx}`);
            return;
          }
          if (status.status === "failed") {
            throw new Error(status.error || "Signing failed");
          }
          await new Promise((r) => setTimeout(r, 1000));
          attempts++;
        }
        throw new Error("Signing timeout");
      } else {
        throw new Error("Unexpected response status");
      }
    } catch (err) {
      setError("Sign failed: " + (err instanceof Error ? err.message : String(err)));
    } finally {
      setSigning(false);
    }
  };

  return (
    <div className="sign-flow-container">
      <h1>Sign Memory</h1>
      
      <textarea
        name="content"
        value={content}
        onChange={(e) => setContent(e.target.value)}
        placeholder="Enter memory content"
        rows={5}
        disabled={signing}
      />

      <button name="sign-memory" onClick={handleSignMemory} disabled={signing}>
        {signing ? "Signing..." : "Sign Memory"}
      </button>

      {error && <div className="error">{error}</div>}
      {result && <div id="sign-result">{result}</div>}
    </div>
  );
};

export default SignFlowPage;