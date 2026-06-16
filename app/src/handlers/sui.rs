use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use std::sync::Arc;

use crate::{
    models::{
        ApiResponse, WalletAction, WalletActionConfig, WalletDashboardQuery,
        WalletDashboardResponse,
    },
    state::AppState,
};

type AppResult<T> = Result<Json<ApiResponse<T>>, (StatusCode, Json<ApiResponse<()>>)>;

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (StatusCode::BAD_REQUEST, Json(ApiResponse::err(msg)))
}

fn internal_error(msg: impl Into<String>) -> (StatusCode, Json<ApiResponse<()>>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiResponse::err(msg)),
    )
}

#[derive(Debug, Serialize)]
pub struct SuiConfigResponse {
    pub network: String,
    pub rpc_url: String,
    pub package_id: Option<String>,
    pub wager_package_id: Option<String>,
    pub usdc_coin_type: Option<String>,
    pub staking_module: String,
    pub wager_module: String,
}

#[derive(Debug, Serialize)]
pub struct SuiAppConfigResponse {
    pub active_network: String,
    pub networks: Vec<SuiConfigResponse>,
}

#[derive(Debug, Serialize)]
pub struct SuiHealthResponse {
    #[serde(flatten)]
    pub config: SuiConfigResponse,
    pub chain_identifier: String,
    pub reference_gas_price: serde_json::Value,
}

pub async fn get_sui_config(State(state): State<Arc<AppState>>) -> AppResult<SuiAppConfigResponse> {
    let config = state.sui.config();
    Ok(Json(ApiResponse::ok(SuiAppConfigResponse {
        active_network: config.active_network.clone(),
        networks: config
            .networks
            .iter()
            .map(sui_config_response)
            .collect::<Vec<_>>(),
    })))
}

pub async fn get_sui_network_config(
    State(state): State<Arc<AppState>>,
    Path(network): Path<String>,
) -> AppResult<SuiConfigResponse> {
    let config = state
        .sui
        .config()
        .network(&network)
        .ok_or_else(|| bad_request("Unsupported Sui network"))?;

    Ok(Json(ApiResponse::ok(sui_config_response(config))))
}

pub async fn get_sui_health(State(state): State<Arc<AppState>>) -> AppResult<SuiHealthResponse> {
    let config = state.sui.active_config();
    let chain_identifier = state
        .sui
        .chain_identifier()
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    let reference_gas_price = state
        .sui
        .reference_gas_price()
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(SuiHealthResponse {
        config: sui_config_response(config),
        chain_identifier,
        reference_gas_price,
    })))
}

pub async fn get_sui_network_health(
    State(state): State<Arc<AppState>>,
    Path(network): Path<String>,
) -> AppResult<SuiHealthResponse> {
    get_network_health(&state, &network).await
}

pub async fn get_wallet_balances(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
) -> AppResult<Vec<crate::services::sui::SuiBalance>> {
    if crate::services::sui::SuiService::normalize_address(&wallet).is_none() {
        return Err(bad_request("Invalid Sui wallet address"));
    }

    let balances = state
        .sui
        .all_balances(&wallet)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(balances)))
}

pub async fn get_wallet_usdc_balance(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
) -> AppResult<crate::services::sui::SuiCoinBalance> {
    if crate::services::sui::SuiService::normalize_address(&wallet).is_none() {
        return Err(bad_request("Invalid Sui wallet address"));
    }

    let balance = state
        .sui
        .usdc_balance(&wallet)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(balance)))
}

pub async fn get_network_wallet_balances_handler(
    State(state): State<Arc<AppState>>,
    Path((network, wallet)): Path<(String, String)>,
) -> AppResult<Vec<crate::services::sui::SuiBalance>> {
    get_network_wallet_balances(&state, &network, &wallet).await
}

pub async fn get_network_wallet_usdc_balance_handler(
    State(state): State<Arc<AppState>>,
    Path((network, wallet)): Path<(String, String)>,
) -> AppResult<crate::services::sui::SuiCoinBalance> {
    get_network_wallet_usdc_balance(&state, &network, &wallet).await
}

pub async fn get_wallet_dashboard(
    State(state): State<Arc<AppState>>,
    Path(wallet): Path<String>,
    Query(query): Query<WalletDashboardQuery>,
) -> AppResult<WalletDashboardResponse> {
    let network = state.sui.active_config().network.clone();
    get_network_wallet_dashboard(&state, &network, &wallet, &query).await
}

pub async fn get_network_wallet_dashboard_handler(
    State(state): State<Arc<AppState>>,
    Path((network, wallet)): Path<(String, String)>,
    Query(query): Query<WalletDashboardQuery>,
) -> AppResult<WalletDashboardResponse> {
    get_network_wallet_dashboard(&state, &network, &wallet, &query).await
}

async fn get_network_health(state: &Arc<AppState>, network: &str) -> AppResult<SuiHealthResponse> {
    let config = state
        .sui
        .config()
        .network(network)
        .ok_or_else(|| bad_request("Unsupported Sui network"))?;

    let chain_identifier = state
        .sui
        .chain_identifier_for(&config.network)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    let reference_gas_price = state
        .sui
        .reference_gas_price_for(&config.network)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(SuiHealthResponse {
        config: sui_config_response(config),
        chain_identifier,
        reference_gas_price,
    })))
}

async fn get_network_wallet_balances(
    state: &Arc<AppState>,
    network: &str,
    wallet: &str,
) -> AppResult<Vec<crate::services::sui::SuiBalance>> {
    if state.sui.config().network(network).is_none() {
        return Err(bad_request("Unsupported Sui network"));
    }

    if crate::services::sui::SuiService::normalize_address(wallet).is_none() {
        return Err(bad_request("Invalid Sui wallet address"));
    }

    let balances = state
        .sui
        .all_balances_for(network, wallet)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(balances)))
}

async fn get_network_wallet_usdc_balance(
    state: &Arc<AppState>,
    network: &str,
    wallet: &str,
) -> AppResult<crate::services::sui::SuiCoinBalance> {
    if state.sui.config().network(network).is_none() {
        return Err(bad_request("Unsupported Sui network"));
    }

    if crate::services::sui::SuiService::normalize_address(wallet).is_none() {
        return Err(bad_request("Invalid Sui wallet address"));
    }

    let balance = state
        .sui
        .usdc_balance_for(network, wallet)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(balance)))
}

async fn get_network_wallet_dashboard(
    state: &Arc<AppState>,
    network: &str,
    wallet: &str,
    query: &WalletDashboardQuery,
) -> AppResult<WalletDashboardResponse> {
    let config = state
        .sui
        .config()
        .network(network)
        .ok_or_else(|| bad_request("Unsupported Sui network"))?;

    let wallet = crate::services::sui::SuiService::normalize_address(wallet)
        .ok_or_else(|| bad_request("Invalid Sui wallet address"))?;

    let balance = state
        .sui
        .usdc_balance_for(&config.network, &wallet)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    let available_balance_usdc = balance.total_balance.parse::<i64>().unwrap_or(0);

    let locked_in_kombats_usdc = state
        .db
        .get_locked_in_kombats_usdc(&wallet)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    let transaction_history = state
        .db
        .list_wallet_transactions(&wallet, query.limit, query.offset)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(ApiResponse::ok(WalletDashboardResponse {
        network: config.network.clone(),
        wallet,
        usdc_coin_type: config.usdc_coin_type.clone(),
        available_balance_usdc,
        locked_in_kombats_usdc,
        total_balance_usdc: available_balance_usdc + locked_in_kombats_usdc,
        transaction_history,
        actions: WalletActionConfig {
            fund_wallet: WalletAction {
                enabled: state.ramp.config().dynamic_onramp_enabled,
                provider: state.ramp.config().primary_provider.clone(),
                requires_frontend_wallet: true,
            },
            withdraw: WalletAction {
                enabled: false,
                provider: "not_supported".to_string(),
                requires_frontend_wallet: true,
            },
        },
    })))
}

fn sui_config_response(config: &crate::services::sui::SuiNetworkConfig) -> SuiConfigResponse {
    SuiConfigResponse {
        network: config.network.clone(),
        rpc_url: config.rpc_url.clone(),
        package_id: config.package_id.clone(),
        wager_package_id: config.wager_package_id.clone(),
        usdc_coin_type: config.usdc_coin_type.clone(),
        staking_module: config.staking_module.clone(),
        wager_module: config.wager_module.clone(),
    }
}
