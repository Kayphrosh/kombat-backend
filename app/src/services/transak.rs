use anyhow::{anyhow, Result};
use serde::Serialize;

use crate::models::{TransakQuoteRequest, TransakQuoteResponse, TransakWidgetRequest};

const TRANSAK_PRODUCTION_WIDGET_URL: &str = "https://global.transak.com";
const TRANSAK_STAGING_WIDGET_URL: &str = "https://global-stg.transak.com";
const TRANSAK_PRODUCTION_API_URL: &str = "https://api.transak.com";
const TRANSAK_STAGING_API_URL: &str = "https://api-stg.transak.com";

#[derive(Debug, Clone, Serialize)]
pub struct TransakConfig {
    pub enabled: bool,
    pub environment: String,
    pub widget_url: String,
    pub api_url: String,
    pub referrer_domain: String,
    pub default_network: String,
    pub default_crypto_currency: String,
    pub default_fiat_currency: String,
    pub partner_fee_bps: u16,
}

impl TransakConfig {
    pub fn from_env() -> Self {
        let environment =
            std::env::var("TRANSAK_ENVIRONMENT").unwrap_or_else(|_| "production".to_string());
        let is_staging = matches!(
            environment.to_ascii_lowercase().as_str(),
            "staging" | "sandbox"
        );

        Self {
            enabled: std::env::var("TRANSAK_API_KEY").is_ok(),
            environment: if is_staging {
                "staging".to_string()
            } else {
                "production".to_string()
            },
            widget_url: std::env::var("TRANSAK_WIDGET_URL").unwrap_or_else(|_| {
                if is_staging {
                    TRANSAK_STAGING_WIDGET_URL.to_string()
                } else {
                    TRANSAK_PRODUCTION_WIDGET_URL.to_string()
                }
            }),
            api_url: std::env::var("TRANSAK_API_URL").unwrap_or_else(|_| {
                if is_staging {
                    TRANSAK_STAGING_API_URL.to_string()
                } else {
                    TRANSAK_PRODUCTION_API_URL.to_string()
                }
            }),
            referrer_domain: std::env::var("TRANSAK_REFERRER_DOMAIN")
                .unwrap_or_else(|_| "localhost".to_string()),
            default_network: std::env::var("TRANSAK_DEFAULT_NETWORK")
                .unwrap_or_else(|_| "sui".to_string()),
            default_crypto_currency: std::env::var("TRANSAK_DEFAULT_CRYPTO_CURRENCY")
                .unwrap_or_else(|_| "USDC".to_string()),
            default_fiat_currency: std::env::var("TRANSAK_DEFAULT_FIAT_CURRENCY")
                .unwrap_or_else(|_| "USD".to_string()),
            partner_fee_bps: std::env::var("TRANSAK_PARTNER_FEE_BPS")
                .ok()
                .and_then(|v| v.parse::<u16>().ok())
                .unwrap_or(0),
        }
    }
}

#[derive(Clone)]
pub struct TransakService {
    client: reqwest::Client,
    config: TransakConfig,
    api_key: Option<String>,
    partner_api_key: Option<String>,
}

impl TransakService {
    pub fn new(config: TransakConfig) -> Self {
        let api_key = std::env::var("TRANSAK_API_KEY").ok();
        let partner_api_key = std::env::var("TRANSAK_PARTNER_API_KEY")
            .ok()
            .or_else(|| api_key.clone());

        Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("failed to build Transak HTTP client"),
            config,
            api_key,
            partner_api_key,
        }
    }

    pub fn config(&self) -> &TransakConfig {
        &self.config
    }

    pub fn widget_url(&self, req: &TransakWidgetRequest) -> Result<String> {
        let api_key = self
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow!("Transak is not configured"))?;

        let product = normalize_product(req.product.as_deref())?;
        let network = req
            .network
            .as_deref()
            .unwrap_or(&self.config.default_network)
            .to_ascii_lowercase();
        let crypto_currency = req
            .crypto_currency_code
            .as_deref()
            .unwrap_or(&self.config.default_crypto_currency)
            .to_ascii_uppercase();
        let fiat_currency = req
            .fiat_currency
            .as_deref()
            .unwrap_or(&self.config.default_fiat_currency)
            .to_ascii_uppercase();

        let mut params = vec![
            ("apiKey".to_string(), api_key.to_string()),
            (
                "referrerDomain".to_string(),
                self.config.referrer_domain.clone(),
            ),
            ("productsAvailed".to_string(), product.to_string()),
            ("network".to_string(), network),
            ("cryptoCurrencyCode".to_string(), crypto_currency),
            ("fiatCurrency".to_string(), fiat_currency),
            ("walletAddress".to_string(), req.wallet_address.clone()),
            ("disableWalletAddressForm".to_string(), "true".to_string()),
            ("isFeeCalculationHidden".to_string(), "false".to_string()),
            ("hideMenu".to_string(), "true".to_string()),
        ];

        if self.config.environment == "staging" {
            params.push(("environment".to_string(), "STAGING".to_string()));
        }
        if let Some(amount) = req.fiat_amount {
            params.push(("fiatAmount".to_string(), amount.to_string()));
        }
        if let Some(amount) = req.crypto_amount {
            params.push(("cryptoAmount".to_string(), amount.to_string()));
        }
        if let Some(method) = &req.payment_method {
            params.push(("paymentMethod".to_string(), method.clone()));
        }
        if let Some(email) = &req.email {
            params.push(("email".to_string(), email.clone()));
        }
        if let Some(order_id) = &req.partner_order_id {
            params.push(("partnerOrderId".to_string(), order_id.clone()));
        }
        if let Some(customer_id) = &req.partner_customer_id {
            params.push(("partnerCustomerId".to_string(), customer_id.clone()));
        }
        if let Some(redirect_url) = &req.redirect_url {
            params.push(("redirectURL".to_string(), redirect_url.clone()));
        }

        Ok(format!(
            "{}?{}",
            self.config.widget_url.trim_end_matches('/'),
            encode_query(&params)
        ))
    }

    pub async fn quote(&self, req: &TransakQuoteRequest) -> Result<TransakQuoteResponse> {
        let partner_api_key = self
            .partner_api_key
            .as_deref()
            .ok_or_else(|| anyhow!("Transak quote API is not configured"))?;

        let product = normalize_product(req.product.as_deref())?;
        let fiat_currency = req
            .fiat_currency
            .as_deref()
            .unwrap_or(&self.config.default_fiat_currency)
            .to_ascii_uppercase();
        let crypto_currency = req
            .crypto_currency_code
            .as_deref()
            .unwrap_or(&self.config.default_crypto_currency)
            .to_ascii_uppercase();
        let network = req
            .network
            .as_deref()
            .unwrap_or(&self.config.default_network)
            .to_ascii_lowercase();

        let mut params = vec![
            ("partnerApiKey".to_string(), partner_api_key.to_string()),
            ("fiatCurrency".to_string(), fiat_currency),
            ("cryptoCurrency".to_string(), crypto_currency),
            ("network".to_string(), network),
            ("isBuyOrSell".to_string(), product.to_string()),
        ];

        if let Some(amount) = req.fiat_amount {
            params.push(("fiatAmount".to_string(), amount.to_string()));
        }
        if let Some(amount) = req.crypto_amount {
            params.push(("cryptoAmount".to_string(), amount.to_string()));
        }
        if let Some(method) = &req.payment_method {
            params.push(("paymentMethod".to_string(), method.clone()));
        }
        if let Some(country_code) = &req.country_code {
            params.push(("countryCode".to_string(), country_code.to_ascii_uppercase()));
        }

        let url = format!(
            "{}/api/v1/pricing/public/quotes?{}",
            self.config.api_url.trim_end_matches('/'),
            encode_query(&params)
        );

        let quote = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| anyhow!("Transak quote request failed: {}", e))?;

        if !quote.status().is_success() {
            return Err(anyhow!(
                "Transak quote API returned HTTP {}",
                quote.status()
            ));
        }

        let raw: serde_json::Value = quote
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse Transak quote response: {}", e))?;

        Ok(TransakQuoteResponse { raw })
    }
}

fn normalize_product(product: Option<&str>) -> Result<&'static str> {
    match product.unwrap_or("BUY").to_ascii_uppercase().as_str() {
        "BUY" => Ok("BUY"),
        _ => Err(anyhow!("Only BUY on-ramp requests are supported")),
    }
}

fn encode_query(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{percent_encode, TransakConfig, TransakService};
    use crate::models::TransakWidgetRequest;
    use rust_decimal::Decimal;

    #[test]
    fn percent_encodes_query_values() {
        assert_eq!(
            percent_encode("https://app.kombat.example/callback?a=1"),
            "https%3A%2F%2Fapp.kombat.example%2Fcallback%3Fa%3D1"
        );
    }

    #[test]
    fn builds_locked_widget_url() {
        std::env::set_var("TRANSAK_API_KEY", "test-key");
        let service = TransakService::new(TransakConfig {
            enabled: true,
            environment: "production".to_string(),
            widget_url: "https://global.transak.com".to_string(),
            api_url: "https://api.transak.com".to_string(),
            referrer_domain: "app.example.com".to_string(),
            default_network: "sui".to_string(),
            default_crypto_currency: "USDC".to_string(),
            default_fiat_currency: "USD".to_string(),
            partner_fee_bps: 0,
        });

        let url =
            service
                .widget_url(&TransakWidgetRequest {
                    wallet_address:
                        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .to_string(),
                    product: Some("BUY".to_string()),
                    fiat_currency: Some("USD".to_string()),
                    fiat_amount: Some(Decimal::new(5000, 2)),
                    crypto_currency_code: Some("USDC".to_string()),
                    crypto_amount: None,
                    network: Some("sui".to_string()),
                    payment_method: Some("credit_debit_card".to_string()),
                    email: Some("user@example.com".to_string()),
                    partner_order_id: Some("order-1".to_string()),
                    partner_customer_id: Some("customer-1".to_string()),
                    redirect_url: Some("https://app.example.com/wallet".to_string()),
                })
                .unwrap();

        assert!(url.starts_with("https://global.transak.com?"));
        assert!(url.contains("apiKey=test-key"));
        assert!(url.contains("productsAvailed=BUY"));
        assert!(url.contains("network=sui"));
        assert!(url.contains("cryptoCurrencyCode=USDC"));
        assert!(url.contains(
            "walletAddress=0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        assert!(url.contains("disableWalletAddressForm=true"));
        assert!(url.contains("redirectURL=https%3A%2F%2Fapp.example.com%2Fwallet"));

        std::env::remove_var("TRANSAK_API_KEY");
    }
}
