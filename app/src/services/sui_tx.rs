//! Minimal Sui transaction signing + submission for backend-initiated calls
//! (e.g. auto-resolving a P2P wager on-chain).
//!
//! Flow: build the transaction server-side via `unsafe_moveCall` (the fullnode
//! selects gas + object versions and returns BCS `txBytes`), sign the Blake2b-256
//! digest of the intent message with the platform ed25519 key, then submit via
//! `sui_executeTransactionBlock`.

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use blake2::digest::consts::U32;
use blake2::{Blake2b, Digest};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};

type Blake2b256 = Blake2b<U32>;

/// Sui intent prefix for a user transaction: scope=TransactionData(0),
/// version=V0(0), app=Sui(0).
const INTENT_TX: [u8; 3] = [0, 0, 0];
/// Signature scheme flag for ed25519.
const FLAG_ED25519: u8 = 0x00;

#[derive(Clone)]
pub struct PlatformSigner {
    signing_key: SigningKey,
    public_key: [u8; 32],
    address: String,
}

#[derive(Debug, Clone)]
pub struct ExecutedMoveCall {
    pub digest: String,
    pub response: Value,
}

impl PlatformSigner {
    /// Load from `PLATFORM_SIGNER_KEYPAIR` — a JSON array of 64 bytes
    /// (`[priv32 || pub32]`, Solana-style). Returns None if unset/invalid.
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var("PLATFORM_SIGNER_KEYPAIR").ok()?;
        let bytes: Vec<u8> = serde_json::from_str(&raw).ok()?;
        if bytes.len() != 64 {
            tracing::warn!(
                "PLATFORM_SIGNER_KEYPAIR must be 64 bytes; got {}",
                bytes.len()
            );
            return None;
        }
        let mut priv32 = [0u8; 32];
        priv32.copy_from_slice(&bytes[0..32]);
        let signing_key = SigningKey::from_bytes(&priv32);
        let public_key = signing_key.verifying_key().to_bytes();

        // Sui address = blake2b256(flag || pubkey)[..32]
        let mut hasher = Blake2b256::new();
        hasher.update([FLAG_ED25519]);
        hasher.update(public_key);
        let digest = hasher.finalize();
        let address = format!("0x{}", hex::encode(digest));

        Some(Self {
            signing_key,
            public_key,
            address,
        })
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    /// Sui serialized signature: base64(flag || sig(64) || pubkey(32)).
    fn serialize_signature(&self, tx_bytes: &[u8]) -> String {
        let mut intent_msg = Vec::with_capacity(3 + tx_bytes.len());
        intent_msg.extend_from_slice(&INTENT_TX);
        intent_msg.extend_from_slice(tx_bytes);

        let mut hasher = Blake2b256::new();
        hasher.update(&intent_msg);
        let digest = hasher.finalize();

        let sig = self.signing_key.sign(&digest);

        let mut out = Vec::with_capacity(1 + 64 + 32);
        out.push(FLAG_ED25519);
        out.extend_from_slice(&sig.to_bytes());
        out.extend_from_slice(&self.public_key);
        B64.encode(out)
    }

    /// Build, sign and execute a Move call. Returns the transaction digest.
    pub async fn move_call_execute(
        &self,
        client: &reqwest::Client,
        rpc_url: &str,
        package: &str,
        module: &str,
        function: &str,
        type_args: Vec<String>,
        args: Vec<Value>,
        gas_budget: u64,
    ) -> Result<String> {
        Ok(self
            .move_call_execute_detailed(
                client, rpc_url, package, module, function, type_args, args, gas_budget,
            )
            .await?
            .digest)
    }

    /// Build, sign and execute a Move call. Returns the digest and full RPC
    /// response so callers can inspect object changes emitted by the tx.
    pub async fn move_call_execute_detailed(
        &self,
        client: &reqwest::Client,
        rpc_url: &str,
        package: &str,
        module: &str,
        function: &str,
        type_args: Vec<String>,
        args: Vec<Value>,
        gas_budget: u64,
    ) -> Result<ExecutedMoveCall> {
        // 1) Build the transaction (fullnode selects gas + object refs).
        let build = rpc(
            client,
            rpc_url,
            "unsafe_moveCall",
            json!([
                self.address,
                package,
                module,
                function,
                type_args,
                args,
                Value::Null, // gas object — let the node pick
                gas_budget.to_string(),
                Value::Null, // execution mode
            ]),
        )
        .await?;

        let tx_bytes_b64 = build
            .get("txBytes")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("unsafe_moveCall returned no txBytes: {}", build))?;
        let tx_bytes = B64.decode(tx_bytes_b64).context("decode txBytes")?;

        // 2) Sign.
        let signature = self.serialize_signature(&tx_bytes);

        // 3) Execute.
        let exec = rpc(
            client,
            rpc_url,
            "sui_executeTransactionBlock",
            json!([
                tx_bytes_b64,
                [signature],
                { "showEffects": true, "showObjectChanges": true },
                "WaitForLocalExecution"
            ]),
        )
        .await?;

        let status = exec
            .pointer("/effects/status/status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if status != "success" {
            return Err(anyhow!(
                "transaction did not succeed ({}): {}",
                status,
                exec
            ));
        }

        let digest = exec
            .get("digest")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("execute returned no digest: {}", exec))?;
        Ok(ExecutedMoveCall {
            digest: digest.to_string(),
            response: exec,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The 64-byte keypair currently in .env and its known Sui ed25519 address.
    const KEYPAIR: [u8; 64] = [
        23, 51, 87, 62, 236, 68, 49, 254, 50, 243, 231, 232, 47, 10, 93, 111, 208, 175, 251, 2, 85,
        119, 116, 69, 219, 68, 254, 33, 133, 121, 250, 203, 109, 157, 153, 91, 236, 47, 226, 231,
        103, 89, 9, 34, 147, 33, 202, 15, 91, 162, 145, 172, 22, 239, 42, 33, 39, 134, 161, 164,
        155, 41, 205, 32,
    ];

    // Live testnet round-trip of the build→sign→submit pipeline (the same path
    // resolve_wager uses). Requires the funded platform signer + its USDC coin.
    // Run with: cargo test e2e_create_and_cancel_wager -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn e2e_create_and_cancel_wager() {
        std::env::set_var(
            "PLATFORM_SIGNER_KEYPAIR",
            serde_json::to_string(&KEYPAIR.to_vec()).unwrap(),
        );
        let signer = PlatformSigner::from_env().unwrap();
        let client = reqwest::Client::new();
        let rpc_url = "https://fullnode.testnet.sui.io:443";
        let package = "0xae1b02ef5fdabec4d8d508d08a43c077296ccd6dd273ba09bcad24bec987e2ea";
        let usdc = "0xa1ec7fc00a6f40db9693ad1415d0c193ad3906494428cf252621037bd7117e29::usdc::USDC";
        let usdc_coin = "0x7234c7bc7b9d6a6dde803b4e164fb4689bc565422644ca8e776fb1ee2e79a81a";
        let me = signer.address().to_string();

        let create_digest = signer
            .move_call_execute(
                &client,
                rpc_url,
                package,
                "wager",
                "create_wager",
                vec![usdc.to_string()],
                vec![
                    json!(usdc_coin),
                    json!("e2e test wager"),
                    json!(""),
                    json!("0x0000000000000000000000000000000000000000000000000000000000000000"),
                    json!("99999999999999"),
                    json!(me),
                    json!("0x6"),
                ],
                100_000_000,
            )
            .await
            .expect("create_wager");
        println!("create_wager digest: {}", create_digest);

        // Find the shared Wager object created by the tx.
        let tx = rpc(
            &client,
            rpc_url,
            "sui_getTransactionBlock",
            json!([create_digest, { "showObjectChanges": true }]),
        )
        .await
        .unwrap();
        let wager_id = tx
            .get("objectChanges")
            .and_then(Value::as_array)
            .and_then(|cs| {
                cs.iter().find(|c| {
                    c.get("type").and_then(Value::as_str) == Some("created")
                        && c.get("objectType")
                            .and_then(Value::as_str)
                            .map(|t| t.contains("::wager::Wager"))
                            .unwrap_or(false)
                })
            })
            .and_then(|c| c.get("objectId").and_then(Value::as_str))
            .expect("created Wager object")
            .to_string();
        println!("wager object: {}", wager_id);

        let cancel_digest = signer
            .move_call_execute(
                &client,
                rpc_url,
                package,
                "wager",
                "cancel_wager",
                vec![usdc.to_string()],
                vec![json!(wager_id), json!("0x6")],
                100_000_000,
            )
            .await
            .expect("cancel_wager");
        println!("cancel_wager digest: {}", cancel_digest);
    }

    #[test]
    fn derives_known_sui_address() {
        std::env::set_var(
            "PLATFORM_SIGNER_KEYPAIR",
            serde_json::to_string(&KEYPAIR.to_vec()).unwrap(),
        );
        let signer = PlatformSigner::from_env().expect("signer loads");
        assert_eq!(
            signer.address(),
            "0xf4f28cf4e69cada95b5402080eff34f9c6320e3738689f0c1c8fa62e723ea373"
        );
    }
}

async fn rpc(
    client: &reqwest::Client,
    rpc_url: &str,
    method: &str,
    params: Value,
) -> Result<Value> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let resp = client.post(rpc_url).json(&body).send().await?;
    let payload: Value = resp.json().await?;
    if let Some(err) = payload.get("error") {
        return Err(anyhow!("{} RPC error: {}", method, err));
    }
    payload
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow!("{} returned no result: {}", method, payload))
}
