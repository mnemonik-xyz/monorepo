//! Solana JSON-RPC client. The x402 payment path uses it to read USDC
//! transfers. Memory records are not written to Solana.

use anyhow::Context;
use solana_sdk::pubkey::Pubkey;

pub struct SolanaClient {
    rpc_url: String,
    client: reqwest::Client,
}

impl SolanaClient {
    pub fn new(rpc_url: &str) -> Self {
        Self {
            rpc_url: rpc_url.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Raw JSON-RPC call. Internal use only — typed methods are the public API surface.
    /// Demoted to `pub(crate)` in Task 20 (CODE-AUDIT-005); all cross-crate callers
    /// should use typed methods instead.
    pub(crate) async fn rpc(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": method, "params": params
        });
        let resp = self.client.post(&self.rpc_url).json(&body).send().await?;
        let result: serde_json::Value = resp.json().await?;
        if let Some(err) = result.get("error") {
            anyhow::bail!("Solana RPC error: {err}");
        }
        Ok(result["result"].clone())
    }

    pub async fn airdrop(&self, pubkey: &Pubkey, lamports: u64) -> anyhow::Result<String> {
        let result = self
            .rpc(
                "requestAirdrop",
                serde_json::json!([pubkey.to_string(), lamports]),
            )
            .await?;
        let sig = result.as_str().context("no airdrop sig")?.to_string();
        self.confirm_tx(&sig).await?;
        Ok(sig)
    }

    pub async fn health_check(&self) -> bool {
        self.rpc("getHealth", serde_json::json!([])).await.is_ok()
    }

    /// Extract the signer pubkeys from a confirmed transaction.
    /// Returns the list of account keys that are marked as signers.
    pub async fn get_tx_signers(&self, tx_sig: &str) -> anyhow::Result<Vec<String>> {
        let result = self.rpc("getTransaction", serde_json::json!([
            tx_sig,
            {"encoding": "jsonParsed", "commitment": "confirmed", "maxSupportedTransactionVersion": 0}
        ])).await?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let mut signers = vec![];

        // jsonParsed format: message.accountKeys is an array of {pubkey, signer, writable, source}
        if let Some(account_keys) = result["transaction"]["message"]["accountKeys"].as_array() {
            for key in account_keys {
                if key["signer"].as_bool() == Some(true) {
                    if let Some(pk) = key["pubkey"].as_str() {
                        signers.push(pk.to_string());
                    }
                }
            }
        }

        Ok(signers)
    }

    pub async fn confirm_tx(&self, sig: &str) -> anyhow::Result<()> {
        for _ in 0..30 {
            let result = self
                .rpc("getSignatureStatuses", serde_json::json!([[sig]]))
                .await?;
            if let Some(statuses) = result["value"].as_array() {
                if let Some(status) = statuses.first().and_then(|s| s.as_object()) {
                    if let Some(conf) = status.get("confirmationStatus").and_then(|c| c.as_str()) {
                        if conf == "confirmed" || conf == "finalized" {
                            if status.get("err").is_none_or(|e| e.is_null()) {
                                return Ok(());
                            }
                            anyhow::bail!("tx failed: {:?}", status.get("err"));
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        anyhow::bail!("tx {sig} not confirmed")
    }
    /// Fetch the token balance delta for `recipient` holding `mint` in a confirmed
    /// transaction. Returns `None` when the tx doesn't exist, failed on-chain, or
    /// has no matching token account change. Returns the amount received (in the
    /// token's smallest unit, e.g. micro-USDC for USDC) when positive.
    ///
    /// This replaces a direct `client.rpc("getTransaction", ...)` call in
    /// `mcp/src/payment.rs::verify_usdc_transfer` (Task 20 / CODE-AUDIT-005).
    /// Core handles RPC mechanics; the USDC-vs-recipient policy stays in mcp.
    pub async fn get_token_balance_delta(
        &self,
        tx_sig: &str,
        recipient: &str,
        mint: &str,
    ) -> anyhow::Result<Option<u64>> {
        let result = self.rpc("getTransaction", serde_json::json!([
            tx_sig,
            {"encoding": "jsonParsed", "commitment": "confirmed", "maxSupportedTransactionVersion": 0}
        ])).await?;

        if result.is_null() {
            return Ok(None);
        }

        if !result["meta"]["err"].is_null() {
            return Ok(None);
        }

        let pre = result["meta"]["preTokenBalances"].as_array();
        let post = result["meta"]["postTokenBalances"].as_array();

        if let (Some(pre_balances), Some(post_balances)) = (pre, post) {
            for post_entry in post_balances {
                let owner = post_entry["owner"].as_str().unwrap_or("");
                let entry_mint = post_entry["mint"].as_str().unwrap_or("");
                if owner != recipient || entry_mint != mint {
                    continue;
                }
                let post_amount: u64 = post_entry["uiTokenAmount"]["amount"]
                    .as_str()
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0);
                let account_index = post_entry["accountIndex"].as_u64().unwrap_or(u64::MAX);
                let pre_amount: u64 = pre_balances
                    .iter()
                    .find(|e| e["accountIndex"].as_u64() == Some(account_index))
                    .and_then(|e| e["uiTokenAmount"]["amount"].as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let delta = post_amount.saturating_sub(pre_amount);
                if delta > 0 {
                    return Ok(Some(delta));
                }
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use solana_sdk::signature::{Keypair, Signer};

    const FAKE_SIG: &str =
        "5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7";

    fn mock_signature_confirmed(server: &MockServer) {
        server.mock(|when, then| {
            when.method(POST).path("/").body_includes("getSignatureStatuses");
            then.status(200).body(
                r#"{"jsonrpc":"2.0","id":1,"result":{"context":{"slot":1},"value":[{"confirmationStatus":"confirmed","err":null,"slot":1,"confirmations":1}]}}"#
            );
        });
    }

    #[tokio::test]
    async fn test_airdrop_success() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/").body_includes("requestAirdrop");
            then.status(200).body(format!(
                r#"{{"jsonrpc":"2.0","id":1,"result":"{FAKE_SIG}"}}"#
            ));
        });
        mock_signature_confirmed(&server);

        let client = SolanaClient::new(&server.base_url());
        let pubkey = Keypair::new().pubkey();
        let result = client.airdrop(&pubkey, 1_000_000).await;

        assert!(result.is_ok(), "airdrop failed: {:?}", result.err());
        assert_eq!(result.unwrap(), FAKE_SIG);
    }

    #[tokio::test]
    async fn test_get_tx_signers() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/").body_includes("getTransaction");
            then.status(200).body(
                r#"{"jsonrpc":"2.0","id":1,"result":{"meta":{"err":null},"transaction":{"message":{"accountKeys":[{"pubkey":"SignerPubkey1111111111111111111111111111111","signer":true,"writable":true,"source":"transaction"},{"pubkey":"NonSignerPubkey11111111111111111111111111111","signer":false,"writable":false,"source":"transaction"}]}},"slot":1}}"#
            );
        });

        let client = SolanaClient::new(&server.base_url());
        let signers = client.get_tx_signers("any_sig").await.unwrap();

        assert_eq!(signers.len(), 1);
        assert_eq!(signers[0], "SignerPubkey1111111111111111111111111111111");
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn test_confirm_tx_retry_exhaustion() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/").body_includes("requestAirdrop");
            then.status(200).body(format!(
                r#"{{"jsonrpc":"2.0","id":1,"result":"{FAKE_SIG}"}}"#
            ));
        });
        // getSignatureStatuses always returns null status (pending) — never confirms
        server.mock(|when, then| {
            when.method(POST)
                .path("/")
                .body_includes("getSignatureStatuses");
            then.status(200)
                .body(r#"{"jsonrpc":"2.0","id":1,"result":{"context":{"slot":1},"value":[null]}}"#);
        });

        let client = SolanaClient::new(&server.base_url());
        let pubkey = Keypair::new().pubkey();
        let result = client.airdrop(&pubkey, 1).await;

        assert!(result.is_err(), "expected confirm exhaustion error, got Ok");
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("not confirmed"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_health_check_ok() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/").body_includes("getHealth");
            then.status(200)
                .body(r#"{"jsonrpc":"2.0","id":1,"result":"ok"}"#);
        });

        let client = SolanaClient::new(&server.base_url());
        assert!(client.health_check().await);
    }

    #[tokio::test]
    async fn test_rpc_error_handling() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/");
            then.status(200).body(
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not found"}}"#,
            );
        });

        // get_tx_signers calls rpc internally — error propagates out via `?`
        let client = SolanaClient::new(&server.base_url());
        let result = client.get_tx_signers("any_sig").await;

        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("Solana RPC error"), "unexpected error: {err}");
    }

    /// Task 20: verify `get_token_balance_delta` returns the correct delta.
    #[tokio::test]
    async fn test_get_token_balance_delta_success() {
        let server = MockServer::start();
        let recipient = "RecipientPubkey11111111111111111111111111111";
        let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        server.mock(|when, then| {
            when.method(POST).path("/").body_includes("getTransaction");
            then.status(200).body(format!(
                r#"{{"jsonrpc":"2.0","id":1,"result":{{"meta":{{"err":null,"preTokenBalances":[{{"accountIndex":1,"mint":"{usdc_mint}","owner":"{recipient}","uiTokenAmount":{{"amount":"1000000","decimals":6}}}}],"postTokenBalances":[{{"accountIndex":1,"mint":"{usdc_mint}","owner":"{recipient}","uiTokenAmount":{{"amount":"2500000","decimals":6}}}}]}},"transaction":{{}},"slot":1}}}}"#
            ));
        });
        let client = SolanaClient::new(&server.base_url());
        let delta = client
            .get_token_balance_delta("any_sig", recipient, usdc_mint)
            .await
            .unwrap();
        assert_eq!(delta, Some(1_500_000));
    }

    #[tokio::test]
    async fn test_get_token_balance_delta_failed_tx() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/").body_includes("getTransaction");
            then.status(200).body(r#"{"jsonrpc":"2.0","id":1,"result":{"meta":{"err":{"InstructionError":[0,"Custom"]}},"transaction":{},"slot":1}}"#);
        });
        let client = SolanaClient::new(&server.base_url());
        let delta = client
            .get_token_balance_delta("fail_sig", "Recip", "Mint")
            .await
            .unwrap();
        assert_eq!(delta, None);
    }

    #[tokio::test]
    async fn test_get_token_balance_delta_missing_tx() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/").body_includes("getTransaction");
            then.status(200)
                .body(r#"{"jsonrpc":"2.0","id":1,"result":null}"#);
        });
        let client = SolanaClient::new(&server.base_url());
        let delta = client
            .get_token_balance_delta("missing_sig", "Recip", "Mint")
            .await
            .unwrap();
        assert_eq!(delta, None);
    }
}
