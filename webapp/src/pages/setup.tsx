import React, { useState } from "react";
import { useNavigate } from "react-router-dom";

const SetupPage: React.FC = () => {
  const navigate = useNavigate();
  const [storageMode, setStorageMode] = useState("local");
  const [paymentMode, setPaymentMode] = useState("none");
  const [storageStatus, setStorageStatus] = useState("");
  const [paymentStatus, setPaymentStatus] = useState("");

  const handleSaveStorageMode = async () => {
    try {
      const response = await fetch("/api/config", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ key: "STORAGE_MODE", value: storageMode }),
      });
      if (response.ok) {
        setStorageStatus("Saved");
        setTimeout(() => setStorageStatus(""), 3000);
      } else {
        setStorageStatus("Save failed");
      }
    } catch (err) {
      setStorageStatus("Save failed");
    }
  };

  const handleSavePaymentMode = async () => {
    try {
      const response = await fetch("/api/config", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ key: "PAYMENT_MODE", value: paymentMode }),
      });
      if (response.ok) {
        setPaymentStatus("Saved");
        setTimeout(() => setPaymentStatus(""), 3000);
      } else {
        setPaymentStatus("Save failed");
      }
    } catch (err) {
      setPaymentStatus("Save failed");
    }
  };

  return (
    <div className="setup-container">
      <h1>System Configuration</h1>

      <div className="config-section">
        <h2>Storage Mode</h2>
        <select value={storageMode} onChange={(e) => setStorageMode(e.target.value)}>
          <option value="local">Local (SQLite)</option>
          <option value="full">Full (Arweave + Solana)</option>
        </select>
        <button name="save-storage-mode" onClick={handleSaveStorageMode}>
          Save
        </button>
        {storageStatus && <span id="storage-mode-status">{storageStatus}</span>}
      </div>

      <div className="config-section">
        <h2>Payment Mode</h2>
        <select value={paymentMode} onChange={(e) => setPaymentMode(e.target.value)}>
          <option value="none">None (Free)</option>
          <option value="balance">Balance (USDC)</option>
        </select>
        <button name="save-payment-mode" onClick={handleSavePaymentMode}>
          Save
        </button>
        {paymentStatus && <span id="payment-mode-status">{paymentStatus}</span>}
      </div>

      <button onClick={() => navigate("/")}>Back to Home</button>
    </div>
  );
};

export default SetupPage;