use anyhow::{anyhow, Result};
use jsonwebtoken::{DecodingKey, Validation, Algorithm, decode};
use serde::Deserialize;
use solana_sdk::signature::Signature;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

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

/// Verify an ed25519 signature where `pubkey_base58` is a base58-encoded
/// public key (Solana-style), `sig_bytes` are the raw 64-byte signature bytes,
/// and `message` is the original message bytes that were signed.
///
/// Uses solana_sdk types to avoid a direct ed25519-dalek dependency.
pub fn verify_ed25519_signature(pubkey_base58: &str, sig_bytes: &[u8], message: &[u8]) -> Result<()> {
    let pubkey = Pubkey::from_str(pubkey_base58)
        .map_err(|e| anyhow!("invalid pubkey: {}", e))?;
    let signature = Signature::try_from(sig_bytes)
        .map_err(|e| anyhow!("invalid signature bytes: {}", e))?;

    if !signature.verify(pubkey.as_ref(), message) {
        return Err(anyhow!("signature verify failed"));
    }
    Ok(())
}
