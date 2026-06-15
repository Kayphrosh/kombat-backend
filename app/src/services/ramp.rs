use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RampConfig {
    pub primary_provider: String,
    pub dynamic_onramp_enabled: bool,
    pub manual_deposit_enabled: bool,
    pub default_network: String,
    pub default_crypto_currency: String,
    pub default_fiat_currency: String,
    pub partner_fee_bps: u16,
}

impl RampConfig {
    pub fn from_env() -> Self {
        Self {
            primary_provider: std::env::var("RAMP_PRIMARY_PROVIDER")
                .unwrap_or_else(|_| "dynamic_native".to_string()),
            dynamic_onramp_enabled: env_bool("DYNAMIC_ONRAMP_ENABLED", true),
            manual_deposit_enabled: env_bool("MANUAL_CRYPTO_DEPOSIT_ENABLED", true),
            default_network: std::env::var("RAMP_DEFAULT_NETWORK")
                .unwrap_or_else(|_| "sui".to_string()),
            default_crypto_currency: std::env::var("RAMP_DEFAULT_CRYPTO_CURRENCY")
                .unwrap_or_else(|_| "USDC".to_string()),
            default_fiat_currency: std::env::var("RAMP_DEFAULT_FIAT_CURRENCY")
                .unwrap_or_else(|_| "USD".to_string()),
            partner_fee_bps: std::env::var("RAMP_PARTNER_FEE_BPS")
                .ok()
                .and_then(|v| v.parse::<u16>().ok())
                .unwrap_or(0),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RampProvider {
    pub provider: String,
    pub label: String,
    pub kind: String,
    pub enabled: bool,
    pub reason: Option<String>,
    pub launch_method: String,
}

#[derive(Clone)]
pub struct RampService {
    config: RampConfig,
}

impl RampService {
    pub fn new(config: RampConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &RampConfig {
        &self.config
    }

    pub fn providers(&self, transak_enabled: bool) -> Vec<RampProvider> {
        let mut providers = vec![
            RampProvider {
                provider: "dynamic_native".to_string(),
                label: "Card / bank transfer".to_string(),
                kind: "onramp".to_string(),
                enabled: self.config.dynamic_onramp_enabled,
                reason: None,
                launch_method: "dynamic_sdk".to_string(),
            },
            RampProvider {
                provider: "manual_crypto_deposit".to_string(),
                label: "Deposit crypto".to_string(),
                kind: "deposit".to_string(),
                enabled: self.config.manual_deposit_enabled,
                reason: None,
                launch_method: "copy_wallet_address".to_string(),
            },
        ];

        if transak_enabled {
            providers.push(RampProvider {
                provider: "transak".to_string(),
                label: "Transak".to_string(),
                kind: "onramp".to_string(),
                enabled: true,
                reason: None,
                launch_method: "widget_url".to_string(),
            });
        }

        providers
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::{RampConfig, RampService};

    fn config() -> RampConfig {
        RampConfig {
            primary_provider: "dynamic_native".to_string(),
            dynamic_onramp_enabled: true,
            manual_deposit_enabled: true,
            default_network: "sui".to_string(),
            default_crypto_currency: "USDC".to_string(),
            default_fiat_currency: "USD".to_string(),
            partner_fee_bps: 0,
        }
    }

    #[test]
    fn hides_transak_when_not_configured() {
        let service = RampService::new(config());
        let providers = service.providers(false);

        assert!(providers.iter().any(|p| p.provider == "dynamic_native"));
        assert!(providers
            .iter()
            .any(|p| p.provider == "manual_crypto_deposit"));
        assert!(!providers.iter().any(|p| p.provider == "transak"));
    }

    #[test]
    fn includes_transak_only_when_enabled() {
        let service = RampService::new(config());
        let providers = service.providers(true);

        assert!(providers
            .iter()
            .any(|p| p.provider == "transak" && p.enabled));
    }
}
