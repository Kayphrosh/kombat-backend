// app/src/services/dynamic.rs
//! Verifies Dynamic SDK JWTs using their JWKS endpoint.
//! Supports both RS256 and ES256 token signatures.

use anyhow::{anyhow, Result};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Claims embedded in the Dynamic SDK JWT.
#[derive(Debug, Deserialize, Clone)]
pub struct DynamicClaims {
    /// Wallet address (may be in `verified_credentials` or top-level)
    pub sub: Option<String>,
    pub email: Option<String>,
    pub environment_id: Option<String>,
    /// Dynamic specific fields
    pub verified_credentials: Option<Vec<DynamicCredential>>,
    pub exp: Option<usize>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DynamicCredential {
    pub address: Option<String>,
    pub chain: Option<String>,
    pub format: Option<String>,
    pub email: Option<String>,
    pub wallet_name: Option<String>,
    pub public_key: Option<String>,
}

/// Check if a string looks like a valid Solana base58 address (32-44 alphanumeric chars).
fn is_solana_address(s: &str) -> bool {
    let len = s.len();
    (32..=44).contains(&len) && s.chars().all(|c| c.is_alphanumeric())
}

impl DynamicClaims {
    /// Extract the primary Solana wallet address from the token.
    /// Prioritises credentials marked with chain="solana", then any blockchain
    /// credential with a valid-looking base58 address. Never falls back to the
    /// Dynamic user UUID (`sub`).
    pub fn wallet_address(&self) -> Option<String> {
        if let Some(creds) = &self.verified_credentials {
            // 1. Prefer a credential explicitly tagged as Solana
            for cred in creds {
                if let Some(chain) = &cred.chain {
                    let chain_lower = chain.to_lowercase();
                    if (chain_lower == "solana" || chain_lower == "sol") {
                        // Try address first, then public_key
                        if let Some(addr) = &cred.address {
                            if is_solana_address(addr) {
                                return Some(addr.clone());
                            }
                        }
                        if let Some(pk) = &cred.public_key {
                            if is_solana_address(pk) {
                                return Some(pk.clone());
                            }
                        }
                    }
                }
            }

            // 2. Fall back to any blockchain credential with a valid base58 address
            for cred in creds {
                let is_blockchain = cred.format.as_deref() == Some("blockchain");
                if let Some(addr) = &cred.address {
                    if is_blockchain && is_solana_address(addr) {
                        return Some(addr.clone());
                    }
                }
            }

            // 3. Last resort: any credential with a valid-looking Solana address
            for cred in creds {
                if let Some(addr) = &cred.address {
                    if is_solana_address(addr) {
                        return Some(addr.clone());
                    }
                }
            }
        }

        // Log for debugging — do NOT fall back to sub (it's a UUID, not a wallet)
        tracing::warn!(
            "No Solana wallet address found in Dynamic token. sub={:?}, creds={:?}",
            self.sub,
            self.verified_credentials
        );
        None
    }

    /// Extract the email from the token.
    pub fn email_address(&self) -> Option<String> {
        if let Some(email) = &self.email {
            return Some(email.clone());
        }
        if let Some(creds) = &self.verified_credentials {
            for cred in creds {
                if let Some(email) = &cred.email {
                    return Some(email.clone());
                }
            }
        }
        None
    }
}

/// JWKS key entry from Dynamic's well-known endpoint.
#[derive(Debug, Deserialize, Clone)]
struct JwksKey {
    kty: String,
    kid: Option<String>,
    alg: Option<String>,
    /// RSA modulus
    n: Option<String>,
    /// RSA exponent
    e: Option<String>,
    /// EC x coordinate
    x: Option<String>,
    /// EC y coordinate
    y: Option<String>,
    /// EC curve
    crv: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwksResponse {
    keys: Vec<JwksKey>,
}

pub struct DynamicService {
    environment_id: String,
    /// Cached JWKS keys
    cached_keys: Arc<RwLock<Vec<JwksKey>>>,
}

impl DynamicService {
    pub fn new(environment_id: String) -> Self {
        Self {
            environment_id,
            cached_keys: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Fetch JWKS keys from Dynamic's endpoint.
    async fn fetch_jwks(&self) -> Result<Vec<JwksKey>> {
        let url = format!(
            "https://app.dynamic.xyz/api/v0/sdk/{}/.well-known/jwks",
            self.environment_id
        );
        let resp = reqwest::get(&url)
            .await
            .map_err(|e| anyhow!("Failed to fetch JWKS: {}", e))?;

        if !resp.status().is_success() {
            return Err(anyhow!("JWKS endpoint returned status: {}", resp.status()));
        }

        let jwks: JwksResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse JWKS response: {}", e))?;

        Ok(jwks.keys)
    }

    /// Refresh and cache JWKS keys.
    async fn refresh_keys(&self) -> Result<()> {
        let keys = self.fetch_jwks().await?;
        let mut cache = self.cached_keys.write().await;
        *cache = keys;
        Ok(())
    }

    /// Verify a Dynamic SDK JWT and return the claims.
    pub async fn verify_token(&self, token: &str) -> Result<DynamicClaims> {
        // Ensure we have keys cached
        {
            let cache = self.cached_keys.read().await;
            if cache.is_empty() {
                drop(cache);
                self.refresh_keys().await?;
            }
        }

        let keys = self.cached_keys.read().await;

        // Try each key until one works
        let mut last_err = anyhow!("no JWKS keys available");
        for key in keys.iter() {
            match self.try_verify_with_key(token, key) {
                Ok(claims) => return Ok(claims),
                Err(e) => last_err = e,
            }
        }

        // Keys might be stale — refresh and retry once
        drop(keys);
        self.refresh_keys().await?;
        let keys = self.cached_keys.read().await;

        for key in keys.iter() {
            match self.try_verify_with_key(token, key) {
                Ok(claims) => return Ok(claims),
                Err(e) => last_err = e,
            }
        }

        Err(anyhow!("Dynamic JWT verification failed: {}", last_err))
    }

    fn try_verify_with_key(&self, token: &str, key: &JwksKey) -> Result<DynamicClaims> {
        let alg_str = key.alg.as_deref().unwrap_or("RS256");

        let (algorithm, decoding_key) = match key.kty.as_str() {
            "RSA" => {
                let n = key.n.as_ref().ok_or_else(|| anyhow!("RSA key missing n"))?;
                let e = key.e.as_ref().ok_or_else(|| anyhow!("RSA key missing e"))?;
                let dk = DecodingKey::from_rsa_components(n, e)
                    .map_err(|e| anyhow!("Invalid RSA components: {}", e))?;
                let alg = match alg_str {
                    "RS384" => Algorithm::RS384,
                    "RS512" => Algorithm::RS512,
                    _ => Algorithm::RS256,
                };
                (alg, dk)
            }
            "EC" => {
                let x = key.x.as_ref().ok_or_else(|| anyhow!("EC key missing x"))?;
                let y = key.y.as_ref().ok_or_else(|| anyhow!("EC key missing y"))?;
                let dk = DecodingKey::from_ec_components(x, y)
                    .map_err(|e| anyhow!("Invalid EC components: {}", e))?;
                let alg = match alg_str {
                    "ES384" => Algorithm::ES384,
                    _ => Algorithm::ES256,
                };
                (alg, dk)
            }
            other => return Err(anyhow!("Unsupported key type: {}", other)),
        };

        let mut validation = Validation::new(algorithm);
        validation.validate_exp = true;
        validation.leeway = 30;
        // Dynamic tokens may not have standard aud/iss — accept any
        validation.set_required_spec_claims::<String>(&[]); // No required claims

        let data = decode::<DynamicClaims>(token, &decoding_key, &validation)
            .map_err(|e| anyhow!("JWT decode error: {}", e))?;

        Ok(data.claims)
    }
}
