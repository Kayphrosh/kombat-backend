use anyhow::{anyhow, Result};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Claims {
    wallet: String,
    exp: usize,
}

/// Verify a HS256 JWT and return the `wallet` claim on success.
pub fn verify_jwt_get_wallet(token: &str, secret: &str) -> Result<String> {
    let decoding_key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::new(Algorithm::HS256);
    // allow some clock skew
    validation.leeway = 10;

    let token_data = decode::<Claims>(token, &decoding_key, &validation)
        .map_err(|e| anyhow!("jwt decode error: {}", e))?;

    Ok(token_data.claims.wallet)
}
