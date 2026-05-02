import React, { useEffect, useState } from "react";

interface BalanceInfo {
  usdc: number;
  arweaveWrites: number;
  solanaAnchors: number;
}

const BalancePage: React.FC = () => {
  const [balance, setBalance] = useState<BalanceInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const fetchBalance = async () => {
      try {
        const response = await fetch("/api/balance", {
          headers: { Authorization: `Bearer ${localStorage.getItem("jwt")}` },
        });
        if (!response.ok) throw new Error("Failed to fetch balance");
        const data = await response.json();
        setBalance(data);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Unknown error");
      } finally {
        setLoading(false);
      }
    };

    fetchBalance();
  }, []);

  const handleTopUp = async () => {
    try {
      const response = await fetch("/api/top-up", {
        method: "POST",
        headers: { 
          "Content-Type": "application/json",
          Authorization: `Bearer ${localStorage.getItem("jwt")}`,
        },
        body: JSON.stringify({ amount: 1.0 }), // Default top-up $1 USDC
      });
      if (!response.ok) throw new Error("Top-up failed");
      const result = await response.json();
      alert(`Top-up successful: ${result.txId}`);
      // Refresh balance
      window.location.reload();
    } catch (err) {
      alert("Top-up failed: " + (err instanceof Error ? err.message : String(err)));
    }
  };

  if (loading) return <div>Loading balance...</div>;
  if (error) return <div>Error: {error}</div>;
  if (!balance) return <div>No balance data</div>;

  return (
    <div className="balance-container">
      <h1>Account Balance</h1>
      
      <div className="balance-info">
        <h2>USDC Balance</h2>
        <p id="usdc-balance">${balance.usdc.toFixed(2)}</p>
        {balance.usdc < 0.1 && (
          <div id="low-balance-warning" className="warning">
            Low balance: Please top up to continue writing attestations
          </div>
        )}
      </div>

      <div className="usage-info">
        <h2>Usage</h2>
        <p>Arweave writes: {balance.arweaveWrites}</p>
        <p>Solana anchors: {balance.solanaAnchors}</p>
      </div>

      <button id="top-up-button" onClick={handleTopUp}>
        Top Up $1 USDC
      </button>
    </div>
  );
};

export default BalancePage;