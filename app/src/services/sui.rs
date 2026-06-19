use anyhow::{anyhow, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};

pub const SUI_MAINNET_RPC_URL: &str = "https://fullnode.mainnet.sui.io:443";
pub const SUI_TESTNET_RPC_URL: &str = "https://fullnode.testnet.sui.io:443";
pub const SUI_DEVNET_RPC_URL: &str = "https://fullnode.devnet.sui.io:443";
pub const SUI_LOCALNET_RPC_URL: &str = "http://127.0.0.1:9000";

pub const SUI_MAINNET_USDC_COIN_TYPE: &str =
    "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC";
pub const SUI_TESTNET_USDC_COIN_TYPE: &str =
    "0xa1ec7fc00a6f40db9693ad1415d0c193ad3906494428cf252621037bd7117e29::usdc::USDC";

#[derive(Debug, Clone, Serialize)]
pub struct SuiNetworkConfig {
    pub network: String,
    pub rpc_url: String,
    pub package_id: Option<String>,
    pub admin_cap_object_id: Option<String>,
    /// Wager lives in its own published package (separate from staking).
    pub wager_package_id: Option<String>,
    pub usdc_coin_type: Option<String>,
    pub staking_module: String,
    pub wager_module: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SuiConfig {
    pub active_network: String,
    pub networks: Vec<SuiNetworkConfig>,
}

impl SuiConfig {
    pub fn from_env() -> Self {
        let active_network =
            canonical_network(&std::env::var("SUI_NETWORK").unwrap_or_else(|_| "testnet".into()))
                .unwrap_or_else(|| "testnet".to_string());

        Self {
            active_network: active_network.clone(),
            networks: vec![
                network_from_env("testnet", &active_network),
                network_from_env("mainnet", &active_network),
            ],
        }
    }

    pub fn active_network(&self) -> &SuiNetworkConfig {
        self.network(&self.active_network)
            .or_else(|| self.network("testnet"))
            .expect("Sui config must include testnet")
    }

    pub fn network(&self, network: &str) -> Option<&SuiNetworkConfig> {
        let network = canonical_network(network)?;
        self.networks
            .iter()
            .find(|config| config.network == network)
    }
}

#[derive(Clone)]
pub struct SuiService {
    client: reqwest::Client,
    config: SuiConfig,
}

#[derive(Debug, Deserialize)]
struct SuiRpcResponse<T> {
    result: Option<T>,
    error: Option<SuiRpcError>,
}

#[derive(Debug, Deserialize)]
struct SuiRpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuiBalance {
    pub coin_type: String,
    pub coin_object_count: u64,
    pub total_balance: String,
    pub locked_balance: Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuiCoinBalance {
    pub coin_type: String,
    pub coin_object_count: u64,
    pub total_balance: String,
    pub locked_balance: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreatedTournamentPool {
    pub digest: String,
    pub pool_object_id: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TournamentPoolSnapshot {
    pub object_type: String,
    pub original_package_id: String,
    pub total_a: i64,
    pub total_b: i64,
    pub vault: i64,
    pub status: u8,
}

#[derive(Debug, Clone)]
pub struct StakePlacedEvent {
    pub tx_digest: String,
    pub owner: String,
    pub match_id: String,
    pub pool_id: String,
    pub receipt_id: String,
    pub outcome: u8,
    pub amount: i64,
    pub total_a: i64,
    pub total_b: i64,
}

impl SuiService {
    pub fn new(config: SuiConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    pub fn config(&self) -> &SuiConfig {
        &self.config
    }

    pub fn active_config(&self) -> &SuiNetworkConfig {
        self.config.active_network()
    }

    /// Submit an on-chain `wager::resolve_wager` call signed by the platform
    /// signer. The signer must equal the wager's on-chain `resolver` and must
    /// hold SUI for gas. Returns the transaction digest on success.
    pub async fn resolve_wager_on_chain(
        &self,
        network: &str,
        wager_object_id: &str,
        winner: &str,
    ) -> anyhow::Result<String> {
        let cfg = self
            .config
            .network(network)
            .ok_or_else(|| anyhow::anyhow!("Unsupported Sui network: {}", network))?;
        let package = cfg
            .wager_package_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("wager_package_id not configured for {}", network))?;
        let coin_type = cfg
            .usdc_coin_type
            .clone()
            .ok_or_else(|| anyhow::anyhow!("usdc_coin_type not configured for {}", network))?;

        let signer = crate::services::sui_tx::PlatformSigner::from_env()
            .ok_or_else(|| anyhow::anyhow!("PLATFORM_SIGNER_KEYPAIR not configured"))?;

        signer
            .move_call_execute(
                &self.client,
                &cfg.rpc_url,
                &package,
                &cfg.wager_module,
                "resolve_wager",
                vec![coin_type],
                vec![
                    serde_json::json!(wager_object_id),
                    serde_json::json!(winner),
                    serde_json::json!("0x6"), // Clock
                ],
                100_000_000, // 0.1 SUI gas budget
            )
            .await
    }

    pub async fn create_tournament_pool_on_chain(
        &self,
        network: &str,
        match_id: &str,
        outcome_a: &str,
        outcome_b: &str,
        stake_deadline_ms: u64,
    ) -> anyhow::Result<CreatedTournamentPool> {
        let cfg = self
            .config
            .network(network)
            .ok_or_else(|| anyhow::anyhow!("Unsupported Sui network: {}", network))?;
        let package = cfg
            .package_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("staking package_id not configured for {}", network))?;
        let admin_cap = cfg.admin_cap_object_id.clone().ok_or_else(|| {
            anyhow::anyhow!("staking admin cap object id not configured for {}", network)
        })?;
        let coin_type = cfg
            .usdc_coin_type
            .clone()
            .ok_or_else(|| anyhow::anyhow!("usdc_coin_type not configured for {}", network))?;

        let signer = crate::services::sui_tx::PlatformSigner::from_env()
            .ok_or_else(|| anyhow::anyhow!("PLATFORM_SIGNER_KEYPAIR not configured"))?;

        let executed = signer
            .move_call_execute_detailed(
                &self.client,
                &cfg.rpc_url,
                &package,
                &cfg.staking_module,
                "create_pool",
                vec![coin_type],
                vec![
                    serde_json::json!(admin_cap),
                    serde_json::json!(match_id),
                    serde_json::json!(outcome_a),
                    serde_json::json!(outcome_b),
                    serde_json::json!(stake_deadline_ms.to_string()),
                    serde_json::json!("0x6"),
                ],
                100_000_000,
            )
            .await?;

        let pool_object_id = find_created_object(&executed.response, &format!("::{}::TournamentPool", cfg.staking_module))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "create_pool succeeded but no TournamentPool object was found in object changes: {}",
                    executed.response
                )
            })?;

        Ok(CreatedTournamentPool {
            digest: executed.digest,
            pool_object_id,
        })
    }

    pub async fn tournament_pool_snapshot(
        &self,
        network: &str,
        pool_object_id: &str,
    ) -> anyhow::Result<TournamentPoolSnapshot> {
        let value: Value = self
            .rpc(
                network,
                "sui_getObject",
                json!([
                    pool_object_id,
                    { "showContent": true, "showType": true, "showOwner": true }
                ]),
            )
            .await?;

        parse_pool_snapshot(&value)
    }

    pub async fn stake_events_for_pool(
        &self,
        network: &str,
        pool_object_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<StakePlacedEvent>> {
        let snapshot = self
            .tournament_pool_snapshot(network, pool_object_id)
            .await?;
        let event_type = format!(
            "{}::tournament_staking::StakePlaced",
            snapshot.original_package_id
        );
        let value: Value = self
            .rpc(
                network,
                "suix_queryEvents",
                json!([
                    { "MoveEventType": event_type },
                    Value::Null,
                    limit.clamp(1, 100),
                    true
                ]),
            )
            .await?;

        let events = value
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_stake_placed_event)
            .filter(|event| event.pool_id.eq_ignore_ascii_case(pool_object_id))
            .collect();

        Ok(events)
    }

    pub fn platform_signer_address() -> Option<String> {
        crate::services::sui_tx::PlatformSigner::from_env().map(|s| s.address().to_string())
    }

    pub fn normalize_address(address: &str) -> Option<String> {
        let address = address.trim();
        let hex = address.strip_prefix("0x")?;
        if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        Some(format!("0x{}", hex.to_ascii_lowercase()))
    }

    pub async fn chain_identifier(&self) -> Result<String> {
        self.chain_identifier_for(&self.config.active_network).await
    }

    pub async fn chain_identifier_for(&self, network: &str) -> Result<String> {
        self.rpc(network, "sui_getChainIdentifier", json!([])).await
    }

    pub async fn reference_gas_price(&self) -> Result<Value> {
        self.reference_gas_price_for(&self.config.active_network)
            .await
    }

    pub async fn reference_gas_price_for(&self, network: &str) -> Result<Value> {
        self.rpc(network, "suix_getReferenceGasPrice", json!([]))
            .await
    }

    pub async fn all_balances(&self, owner: &str) -> Result<Vec<SuiBalance>> {
        self.all_balances_for(&self.config.active_network, owner)
            .await
    }

    pub async fn all_balances_for(&self, network: &str, owner: &str) -> Result<Vec<SuiBalance>> {
        let owner = Self::normalize_address(owner).ok_or_else(|| anyhow!("Invalid Sui address"))?;
        self.rpc(network, "suix_getAllBalances", json!([owner]))
            .await
    }

    pub async fn usdc_balance(&self, owner: &str) -> Result<SuiCoinBalance> {
        self.usdc_balance_for(&self.config.active_network, owner)
            .await
    }

    pub async fn usdc_balance_for(&self, network: &str, owner: &str) -> Result<SuiCoinBalance> {
        let owner = Self::normalize_address(owner).ok_or_else(|| anyhow!("Invalid Sui address"))?;
        let config = self
            .config
            .network(network)
            .ok_or_else(|| anyhow!("Unsupported Sui network: {}", network))?;
        let coin_type = config
            .usdc_coin_type
            .as_deref()
            .ok_or_else(|| anyhow!("USDC coin type is not configured for {}", network))?;

        self.rpc(network, "suix_getBalance", json!([owner, coin_type]))
            .await
    }

    async fn rpc<T: DeserializeOwned>(
        &self,
        network: &str,
        method: &str,
        params: Value,
    ) -> Result<T> {
        let config = self
            .config
            .network(network)
            .ok_or_else(|| anyhow!("Unsupported Sui network: {}", network))?;
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let response = self
            .client
            .post(&config.rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Sui RPC request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(anyhow!("Sui RPC returned HTTP {}", response.status()));
        }

        let rpc_response: SuiRpcResponse<T> = response
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse Sui RPC response: {}", e))?;

        if let Some(error) = rpc_response.error {
            return Err(anyhow!(
                "Sui RPC error {} for {}: {}",
                error.code,
                method,
                error.message
            ));
        }

        rpc_response
            .result
            .ok_or_else(|| anyhow!("Sui RPC response for {} had no result", method))
    }
}

fn canonical_network(network: &str) -> Option<String> {
    match network.trim().to_ascii_lowercase().as_str() {
        "mainnet" => Some("mainnet".to_string()),
        "testnet" => Some("testnet".to_string()),
        "devnet" => Some("devnet".to_string()),
        "localnet" => Some("localnet".to_string()),
        _ => None,
    }
}

fn env_first(keys: &[String]) -> Option<String> {
    keys.iter().find_map(|key| std::env::var(key).ok())
}

fn network_from_env(network: &str, active_network: &str) -> SuiNetworkConfig {
    let prefix = format!("SUI_{}", network.to_ascii_uppercase());
    let active_prefix = if network == active_network {
        vec![
            "SUI_RPC_URL".to_string(),
            "SUI_PACKAGE_ID".to_string(),
            "SUI_USDC_COIN_TYPE".to_string(),
        ]
    } else {
        Vec::new()
    };

    let rpc_url = env_first(&[
        format!("{}_RPC_URL", prefix),
        active_prefix.first().cloned().unwrap_or_default(),
    ])
    .unwrap_or_else(|| default_rpc_url(network).to_string());

    let package_id = env_first(&[
        format!("{}_PACKAGE_ID", prefix),
        active_prefix.get(1).cloned().unwrap_or_default(),
    ]);

    let admin_cap_object_id = env_first(&[
        format!("{}_ADMIN_CAP_OBJECT_ID", prefix),
        "SUI_ADMIN_CAP_OBJECT_ID".to_string(),
    ])
    .and_then(|address| SuiService::normalize_address(&address));

    let wager_package_id = env_first(&[
        format!("{}_WAGER_PACKAGE_ID", prefix),
        "SUI_WAGER_PACKAGE_ID".to_string(),
    ]);

    let usdc_coin_type = env_first(&[
        format!("{}_USDC_COIN_TYPE", prefix),
        active_prefix.get(2).cloned().unwrap_or_default(),
    ])
    .or_else(|| default_usdc_coin_type(network).map(str::to_string));

    SuiNetworkConfig {
        network: network.to_string(),
        rpc_url,
        package_id,
        admin_cap_object_id,
        wager_package_id,
        usdc_coin_type,
        staking_module: std::env::var("SUI_STAKING_MODULE")
            .unwrap_or_else(|_| "tournament_staking".to_string()),
        wager_module: std::env::var("SUI_WAGER_MODULE").unwrap_or_else(|_| "wager".to_string()),
    }
}

fn find_created_object(response: &Value, object_type_suffix: &str) -> Option<String> {
    response
        .get("objectChanges")
        .and_then(Value::as_array)?
        .iter()
        .find(|change| {
            change.get("type").and_then(Value::as_str) == Some("created")
                && change
                    .get("objectType")
                    .and_then(Value::as_str)
                    .map(|object_type| object_type.contains(object_type_suffix))
                    .unwrap_or(false)
        })
        .and_then(|change| change.get("objectId").and_then(Value::as_str))
        .map(ToString::to_string)
}

fn parse_pool_snapshot(value: &Value) -> anyhow::Result<TournamentPoolSnapshot> {
    let object_type = value
        .pointer("/data/type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("pool object response had no type: {}", value))?
        .to_string();
    let original_package_id = object_type
        .split("::")
        .next()
        .ok_or_else(|| anyhow!("pool object type is invalid: {}", object_type))?
        .to_string();
    let fields = value
        .pointer("/data/content/fields")
        .ok_or_else(|| anyhow!("pool object response had no fields: {}", value))?;

    Ok(TournamentPoolSnapshot {
        object_type,
        original_package_id,
        total_a: json_i64(fields, "total_a")?,
        total_b: json_i64(fields, "total_b")?,
        vault: json_i64(fields, "vault")?,
        status: json_i64(fields, "status")? as u8,
    })
}

fn parse_stake_placed_event(value: &Value) -> Option<StakePlacedEvent> {
    let parsed = value.get("parsedJson")?;
    Some(StakePlacedEvent {
        tx_digest: value
            .pointer("/id/txDigest")
            .and_then(Value::as_str)?
            .to_string(),
        owner: parsed.get("owner")?.as_str()?.to_string(),
        match_id: parsed.get("match_id")?.as_str()?.to_string(),
        pool_id: parsed.get("pool_id")?.as_str()?.to_string(),
        receipt_id: parsed.get("receipt_id")?.as_str()?.to_string(),
        outcome: json_i64(parsed, "outcome").ok()? as u8,
        amount: json_i64(parsed, "amount").ok()?,
        total_a: json_i64(parsed, "total_a").ok()?,
        total_b: json_i64(parsed, "total_b").ok()?,
    })
}

fn json_i64(value: &Value, key: &str) -> anyhow::Result<i64> {
    let raw = value
        .get(key)
        .ok_or_else(|| anyhow!("missing numeric field {}", key))?;
    if let Some(n) = raw.as_i64() {
        return Ok(n);
    }
    raw.as_str()
        .ok_or_else(|| anyhow!("field {} is not numeric: {}", key, raw))?
        .parse::<i64>()
        .map_err(|e| anyhow!("field {} is invalid: {}", key, e))
}

fn default_rpc_url(network: &str) -> &'static str {
    match network {
        "mainnet" => SUI_MAINNET_RPC_URL,
        "devnet" => SUI_DEVNET_RPC_URL,
        "localnet" => SUI_LOCALNET_RPC_URL,
        _ => SUI_TESTNET_RPC_URL,
    }
}

fn default_usdc_coin_type(network: &str) -> Option<&'static str> {
    match network {
        "mainnet" => Some(SUI_MAINNET_USDC_COIN_TYPE),
        "testnet" => Some(SUI_TESTNET_USDC_COIN_TYPE),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{SuiConfig, SuiService, SUI_MAINNET_USDC_COIN_TYPE, SUI_TESTNET_USDC_COIN_TYPE};

    #[test]
    fn normalizes_valid_sui_address() {
        let address = "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        assert_eq!(
            SuiService::normalize_address(address).as_deref(),
            Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
    }

    #[test]
    fn rejects_invalid_sui_address() {
        assert!(SuiService::normalize_address("0xabc").is_none());
        assert!(SuiService::normalize_address("abc").is_none());
        assert!(SuiService::normalize_address(
            "0xzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
        )
        .is_none());
    }

    #[test]
    fn includes_testnet_and_mainnet_defaults() {
        let config = SuiConfig::from_env();

        assert_eq!(
            config
                .network("testnet")
                .and_then(|n| n.usdc_coin_type.as_deref()),
            Some(SUI_TESTNET_USDC_COIN_TYPE)
        );
        assert_eq!(
            config
                .network("mainnet")
                .and_then(|n| n.usdc_coin_type.as_deref()),
            Some(SUI_MAINNET_USDC_COIN_TYPE)
        );
    }
}
