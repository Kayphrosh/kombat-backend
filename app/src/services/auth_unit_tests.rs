#[cfg(test)]
mod tests {
    use super::super::auth::verify_ed25519_signature;
    use solana_sdk::signer::Signer;

    #[test]
    fn test_verify_ed25519_signature() {
        // Generate a Solana keypair and sign a message
        let kp = solana_sdk::signature::Keypair::new();
        let msg = b"test-nonce-123";
        let sig = kp.sign_message(msg);
        let pubkey_bs58 = kp.pubkey().to_string();
        let sig_bytes = sig.as_ref();

        let res = verify_ed25519_signature(&pubkey_bs58, sig_bytes, msg);
        assert!(res.is_ok());
    }
}
