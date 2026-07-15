//! Solana client — SPL Memo write/read via JSON-RPC.

use anyhow::Context;
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::Message,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use std::str::FromStr;

const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

pub struct SolanaClient {
    rpc_url: String,
    client: reqwest::Client,
}

/// One anchor memo recovered from the wallet's signature history.
#[derive(Debug, Clone)]
pub struct MemoAnchor {
    pub solana_tx: String,
    /// The `a` field — Arweave tx id of the COSE envelope.
    pub arweave_tx: String,
    /// The `h` field — blake3 of the canonical CBOR payload.
    pub content_hash: String,
    /// Unix seconds of the containing block (memo write time).
    pub block_time: Option<i64>,
}

/// Parse a `getSignaturesForAddress` memo entry into `(arweave_tx, hash)`.
/// The RPC prefixes SPL memos with `"[<len>] "`; the payload must be the
/// anchor JSON with string `a` + `h` fields (v2 and v3 both qualify).
/// Anything else — missing memo, other memo programs, unrelated JSON —
/// returns `None`.
fn parse_anchor_memo(memo: Option<&str>) -> Option<(String, String)> {
    let raw = memo?;
    let json_part = match raw.split_once("] ") {
        Some((_, rest)) => rest,
        None => raw,
    };
    let v: serde_json::Value = serde_json::from_str(json_part).ok()?;
    let a = v.get("a")?.as_str()?;
    let h = v.get("h")?.as_str()?;
    if a.is_empty() || h.is_empty() {
        return None;
    }
    Some((a.to_string(), h.to_string()))
}

impl SolanaClient {
    pub fn new(rpc_url: &str) -> Self {
        Self {
            rpc_url: rpc_url.to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub async fn rpc(
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

    pub async fn write_memo(&self, keypair: &Keypair, memo: &str) -> anyhow::Result<String> {
        let sig = self.submit_memo(keypair, memo).await?;
        self.confirm_tx(&sig).await?;
        Ok(sig)
    }

    /// Submit a memo and return its signature before confirmation. Callers
    /// that persist multi-network delivery state must store this signature
    /// before awaiting confirmation so an ambiguous RPC timeout can be
    /// reconciled instead of submitting a duplicate transaction.
    pub async fn submit_memo(&self, keypair: &Keypair, memo: &str) -> anyhow::Result<String> {
        let memo_pid = Pubkey::from_str(MEMO_PROGRAM_ID)?;
        let ix = Instruction {
            program_id: memo_pid,
            accounts: vec![AccountMeta::new(keypair.pubkey(), true)],
            data: memo.as_bytes().to_vec(),
        };

        let blockhash_result = self
            .rpc(
                "getLatestBlockhash",
                serde_json::json!([{"commitment": "confirmed"}]),
            )
            .await?;
        let blockhash_str = blockhash_result["value"]["blockhash"]
            .as_str()
            .context("no blockhash")?;
        let blockhash = Hash::from_str(blockhash_str)?;

        let msg = Message::new_with_blockhash(&[ix], Some(&keypair.pubkey()), &blockhash);
        let mut tx = Transaction::new_unsigned(msg);
        tx.sign(&[keypair], blockhash);

        let tx_bytes = bincode::serialize(&tx)?;
        let tx_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &tx_bytes);

        // preflightCommitment must match the commitment used for
        // getLatestBlockhash above — otherwise the simulator runs
        // against a more conservative (finalized) view that hasn't
        // seen our confirmed blockhash yet, returning BlockhashNotFound.
        let result = self
            .rpc(
                "sendTransaction",
                serde_json::json!([
                    tx_b64,
                    {"encoding": "base64", "preflightCommitment": "confirmed"}
                ]),
            )
            .await?;
        let sig = result.as_str().context("no tx signature")?.to_string();

        Ok(sig)
    }

    pub async fn read_memo(&self, tx_sig: &str) -> anyhow::Result<Option<serde_json::Value>> {
        let result = self.rpc("getTransaction", serde_json::json!([
            tx_sig,
            {"encoding": "jsonParsed", "commitment": "confirmed", "maxSupportedTransactionVersion": 0}
        ])).await?;

        if result.is_null() {
            return Ok(None);
        }

        // Extract memo from log messages
        if let Some(logs) = result["meta"]["logMessages"].as_array() {
            for log in logs {
                if let Some(s) = log.as_str() {
                    if s.contains("Memo") && s.contains("len") {
                        if let (Some(start), Some(end)) = (s.find('"'), s.rfind('"')) {
                            if end > start {
                                let memo_str = &s[start + 1..end];
                                if let Ok(parsed) =
                                    serde_json::from_str::<serde_json::Value>(memo_str)
                                {
                                    return Ok(Some(parsed));
                                }
                                return Ok(Some(serde_json::json!({"raw": memo_str})));
                            }
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Enumerate every anchor memo this wallet ever wrote, oldest-first.
    ///
    /// The authoritative recovery source (recover-traction-from-chain):
    /// gateway GraphQL does NOT index the historical Irys-bundled items,
    /// but each `participate` write also produced an SPL Memo
    /// (`{"h": blake3, "a": arweave_tx, "v": 2|3}`) from the server wallet,
    /// and `getSignaturesForAddress` returns the memo text inline — one
    /// paginated RPC enumerates the full anchored ledger. Non-anchor memos
    /// and memo-less txs (funding, fees) are skipped.
    pub async fn list_memo_anchors(&self, wallet: &str) -> anyhow::Result<Vec<MemoAnchor>> {
        let mut anchors = Vec::new();
        let mut before: Option<String> = None;
        // 1000 sigs/page; 1000 pages is a backstop far above current scale.
        for _ in 0..1000 {
            let mut opts = serde_json::json!({"limit": 1000});
            if let Some(b) = &before {
                opts["before"] = serde_json::Value::String(b.clone());
            }
            let result = self
                .rpc("getSignaturesForAddress", serde_json::json!([wallet, opts]))
                .await?;
            let entries = result.as_array().cloned().unwrap_or_default();
            let page_len = entries.len();
            for e in &entries {
                let Some(sig) = e["signature"].as_str() else {
                    continue;
                };
                if let Some(anchor) = parse_anchor_memo(e["memo"].as_str()) {
                    anchors.push(MemoAnchor {
                        solana_tx: sig.to_string(),
                        arweave_tx: anchor.0,
                        content_hash: anchor.1,
                        block_time: e["blockTime"].as_i64(),
                    });
                }
                before = Some(sig.to_string());
            }
            if page_len < 1000 {
                break;
            }
        }
        // RPC returns newest-first; recovery wants oldest-first.
        anchors.reverse();
        Ok(anchors)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use solana_sdk::signature::Keypair;

    // 32 zero bytes base58-encoded — a valid Hash value for testing.
    const ZERO_BLOCKHASH_B58: &str = "11111111111111111111111111111111";
    const FAKE_SIG: &str =
        "5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7";

    fn mock_blockhash(server: &MockServer) {
        server.mock(|when, then| {
            when.method(POST).path("/").body_includes("getLatestBlockhash");
            then.status(200).body(format!(
                r#"{{"jsonrpc":"2.0","id":1,"result":{{"context":{{"slot":1}},"value":{{"blockhash":"{ZERO_BLOCKHASH_B58}","lastValidBlockHeight":100}}}}}}"#
            ));
        });
    }

    fn mock_send_transaction(server: &MockServer, sig: &str) {
        server.mock(|when, then| {
            when.method(POST).path("/").body_includes("sendTransaction");
            then.status(200)
                .body(format!(r#"{{"jsonrpc":"2.0","id":1,"result":"{sig}"}}"#));
        });
    }

    fn mock_signature_confirmed(server: &MockServer) {
        server.mock(|when, then| {
            when.method(POST).path("/").body_includes("getSignatureStatuses");
            then.status(200).body(
                r#"{"jsonrpc":"2.0","id":1,"result":{"context":{"slot":1},"value":[{"confirmationStatus":"confirmed","err":null,"slot":1,"confirmations":1}]}}"#
            );
        });
    }

    #[tokio::test]
    async fn test_write_memo_success() {
        let server = MockServer::start();
        mock_blockhash(&server);
        mock_send_transaction(&server, FAKE_SIG);
        mock_signature_confirmed(&server);

        let client = SolanaClient::new(&server.base_url());
        let keypair = Keypair::new();
        let result = client.write_memo(&keypair, r#"{"hello":"world"}"#).await;

        assert!(result.is_ok(), "write_memo failed: {:?}", result.err());
        assert_eq!(result.unwrap(), FAKE_SIG);
    }

    #[tokio::test]
    async fn test_read_memo_success() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/").body_includes("getTransaction");
            then.status(200).body(
                r#"{"jsonrpc":"2.0","id":1,"result":{"meta":{"err":null,"logMessages":["Program MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr invoke [1]","Program log: Memo (len 17): \"{\"h\":\"abc\"}\"","Program MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr success"]},"transaction":{},"slot":1}}"#
            );
        });

        let client = SolanaClient::new(&server.base_url());
        let result = client.read_memo("fake_sig").await.unwrap();

        assert!(result.is_some(), "expected Some(memo), got None");
        let memo = result.unwrap();
        assert_eq!(memo["h"].as_str(), Some("abc"));
    }

    #[tokio::test]
    async fn test_read_memo_not_found() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/").body_includes("getTransaction");
            then.status(200)
                .body(r#"{"jsonrpc":"2.0","id":1,"result":null}"#);
        });

        let client = SolanaClient::new(&server.base_url());
        let result = client.read_memo("missing_sig").await.unwrap();
        assert!(result.is_none());
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

        // read_memo calls rpc internally — error propagates out via `?`
        let client = SolanaClient::new(&server.base_url());
        let result = client.read_memo("any_sig").await;

        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("Solana RPC error"), "unexpected error: {err}");
    }

    #[test]
    fn parse_anchor_memo_handles_rpc_prefix_and_versions() {
        // v2 memo as getSignaturesForAddress returns it (with "[len] " prefix).
        let m = parse_anchor_memo(Some(r#"[97] {"h":"abc123","a":"ArTx111","v":2}"#));
        assert_eq!(m, Some(("ArTx111".to_string(), "abc123".to_string())));
        // v3 (embed model field) without prefix.
        let m = parse_anchor_memo(Some(r#"{"h":"def","a":"ArTx222","m":"bge-small","v":3}"#));
        assert_eq!(m, Some(("ArTx222".to_string(), "def".to_string())));
        // Non-anchor memos and absent memos are skipped.
        assert_eq!(parse_anchor_memo(None), None);
        assert_eq!(parse_anchor_memo(Some("[5] hello")), None);
        assert_eq!(
            parse_anchor_memo(Some(r#"{"note":"unrelated json"}"#)),
            None
        );
        assert_eq!(parse_anchor_memo(Some(r#"{"h":"","a":"","v":2}"#)), None);
    }

    #[tokio::test]
    async fn list_memo_anchors_filters_and_orders_oldest_first() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/");
            then.status(200).json_body(serde_json::json!({
                "jsonrpc": "2.0", "id": 1,
                "result": [
                    // newest-first, as the RPC returns them
                    {"signature": "sig-new", "blockTime": 1747756800,
                     "memo": "[60] {\"h\":\"h2\",\"a\":\"ar2\",\"v\":2}"},
                    {"signature": "sig-fund", "blockTime": 1747000000, "memo": null},
                    {"signature": "sig-old", "blockTime": 1746144000,
                     "memo": "[60] {\"h\":\"h1\",\"a\":\"ar1\",\"v\":3}"},
                ]
            }));
        });
        let client = SolanaClient::new(&server.base_url());
        let anchors = client.list_memo_anchors("WalletPubkey111").await.unwrap();
        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors[0].arweave_tx, "ar1"); // oldest first
        assert_eq!(anchors[0].content_hash, "h1");
        assert_eq!(anchors[1].solana_tx, "sig-new");
        assert_eq!(anchors[1].block_time, Some(1747756800));
    }
}
