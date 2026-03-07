#[cfg(test)]
mod tests {
    use super::super::delegation::DelegationService;
    use base64::Engine;
    use solana_sdk::{pubkey::Pubkey, signer::Signer};
    use std::str::FromStr;

    /// Helper: generate a fresh keypair JSON for tests.
    fn test_keypair_json() -> (String, solana_sdk::signature::Keypair) {
        let kp = solana_sdk::signature::Keypair::new();
        let json = serde_json::to_string(&kp.to_bytes().to_vec()).unwrap();
        (json, kp)
    }

    #[test]
    fn test_from_json_keypair_valid() {
        let (json, kp) = test_keypair_json();
        let svc = DelegationService::from_json_keypair(&json).unwrap();
        assert_eq!(svc.pubkey(), kp.pubkey());
    }

    #[test]
    fn test_from_json_keypair_invalid_json() {
        let result = DelegationService::from_json_keypair("not-json");
        assert!(result.is_err());
        let err_msg = format!("{}", result.err().unwrap());
        assert!(err_msg.contains("JSON byte array"));
    }

    #[test]
    fn test_from_json_keypair_wrong_length() {
        let result = DelegationService::from_json_keypair("[1,2,3]");
        assert!(result.is_err());
    }

    #[test]
    fn test_build_approve_tx_returns_base64() {
        let (json, _) = test_keypair_json();
        let svc = DelegationService::from_json_keypair(&json).unwrap();

        let user_wallet = Pubkey::new_unique();
        let user_ata = Pubkey::new_unique();
        let blockhash = solana_sdk::hash::Hash::new_unique();

        let tx_b64 = svc.build_approve_tx(&user_wallet, &user_ata, 100_000_000, blockhash).unwrap();

        // Must be valid base64
        let decoded = base64::engine::general_purpose::STANDARD.decode(&tx_b64).unwrap();
        assert!(!decoded.is_empty());

        // Must deserialize into a Transaction
        let tx: solana_sdk::transaction::Transaction = bincode::deserialize(&decoded).unwrap();
        // Approve tx should have 1 instruction
        assert_eq!(tx.message.instructions.len(), 1);
        // Fee payer should be the user wallet
        assert_eq!(tx.message.account_keys[0], user_wallet);
    }

    #[test]
    fn test_build_revoke_tx_returns_base64() {
        let (json, _) = test_keypair_json();
        let svc = DelegationService::from_json_keypair(&json).unwrap();

        let user_wallet = Pubkey::new_unique();
        let user_ata = Pubkey::new_unique();
        let blockhash = solana_sdk::hash::Hash::new_unique();

        let tx_b64 = svc.build_revoke_tx(&user_wallet, &user_ata, blockhash).unwrap();
        let decoded = base64::engine::general_purpose::STANDARD.decode(&tx_b64).unwrap();
        let tx: solana_sdk::transaction::Transaction = bincode::deserialize(&decoded).unwrap();

        assert_eq!(tx.message.instructions.len(), 1);
        assert_eq!(tx.message.account_keys[0], user_wallet);
    }

    #[test]
    fn test_build_delegated_transfer_is_signed() {
        let (json, kp) = test_keypair_json();
        let svc = DelegationService::from_json_keypair(&json).unwrap();

        let user_ata = Pubkey::new_unique();
        let dest_ata = Pubkey::new_unique();
        let blockhash = solana_sdk::hash::Hash::new_unique();

        let tx = svc.build_delegated_transfer(&user_ata, &dest_ata, 5_000_000, blockhash).unwrap();

        // The platform signer should be the fee payer (first account key)
        assert_eq!(tx.message.account_keys[0], kp.pubkey());
        // Transaction should be signed (1 signature from platform signer)
        assert_eq!(tx.signatures.len(), 1);
        // Signature should not be all zeros (i.e. actually signed)
        assert_ne!(tx.signatures[0], solana_sdk::signature::Signature::default());
    }

    #[test]
    fn test_approve_instruction_data_layout() {
        let (json, kp) = test_keypair_json();
        let svc = DelegationService::from_json_keypair(&json).unwrap();

        let user_wallet = Pubkey::new_unique();
        let user_ata = Pubkey::new_unique();
        let amount: u64 = 250_000_000; // 250 USDC
        let blockhash = solana_sdk::hash::Hash::new_unique();

        let tx_b64 = svc.build_approve_tx(&user_wallet, &user_ata, amount, blockhash).unwrap();
        let decoded = base64::engine::general_purpose::STANDARD.decode(&tx_b64).unwrap();
        let tx: solana_sdk::transaction::Transaction = bincode::deserialize(&decoded).unwrap();

        let ix = &tx.message.instructions[0];
        // Data: [4 (approve tag), amount LE bytes (8)]
        assert_eq!(ix.data[0], 4u8); // SPL Token Approve tag
        let encoded_amount = u64::from_le_bytes(ix.data[1..9].try_into().unwrap());
        assert_eq!(encoded_amount, amount);
    }

    #[test]
    fn test_revoke_instruction_data_layout() {
        let (json, _) = test_keypair_json();
        let svc = DelegationService::from_json_keypair(&json).unwrap();

        let user_wallet = Pubkey::new_unique();
        let user_ata = Pubkey::new_unique();
        let blockhash = solana_sdk::hash::Hash::new_unique();

        let tx_b64 = svc.build_revoke_tx(&user_wallet, &user_ata, blockhash).unwrap();
        let decoded = base64::engine::general_purpose::STANDARD.decode(&tx_b64).unwrap();
        let tx: solana_sdk::transaction::Transaction = bincode::deserialize(&decoded).unwrap();

        let ix = &tx.message.instructions[0];
        // Data: [5 (revoke tag)] — just the tag byte
        assert_eq!(ix.data.len(), 1);
        assert_eq!(ix.data[0], 5u8); // SPL Token Revoke tag
    }

    #[test]
    fn test_transfer_instruction_data_layout() {
        let (json, _) = test_keypair_json();
        let svc = DelegationService::from_json_keypair(&json).unwrap();

        let user_ata = Pubkey::new_unique();
        let dest_ata = Pubkey::new_unique();
        let amount: u64 = 10_000_000; // 10 USDC
        let blockhash = solana_sdk::hash::Hash::new_unique();

        let tx = svc.build_delegated_transfer(&user_ata, &dest_ata, amount, blockhash).unwrap();

        let ix = &tx.message.instructions[0];
        assert_eq!(ix.data[0], 3u8); // SPL Token Transfer tag
        let encoded_amount = u64::from_le_bytes(ix.data[1..9].try_into().unwrap());
        assert_eq!(encoded_amount, amount);
    }

    #[test]
    fn test_transfer_accounts_correct() {
        let (json, kp) = test_keypair_json();
        let svc = DelegationService::from_json_keypair(&json).unwrap();

        let user_ata = Pubkey::new_unique();
        let dest_ata = Pubkey::new_unique();
        let blockhash = solana_sdk::hash::Hash::new_unique();

        let tx = svc.build_delegated_transfer(&user_ata, &dest_ata, 1_000_000, blockhash).unwrap();

        let ix = &tx.message.instructions[0];
        let keys = &tx.message.account_keys;

        // Account 0 = platform signer (fee payer), also used as delegate authority
        assert_eq!(keys[0], kp.pubkey());
        // The instruction accounts should reference source, dest, and delegate
        // ix.accounts are indices into account_keys
        let source_idx = ix.accounts[0] as usize;
        let dest_idx = ix.accounts[1] as usize;
        let delegate_idx = ix.accounts[2] as usize;

        assert_eq!(keys[source_idx], user_ata);
        assert_eq!(keys[dest_idx], dest_ata);
        assert_eq!(keys[delegate_idx], kp.pubkey()); // delegate is the platform signer
    }

    #[test]
    fn test_max_delegation_constant() {
        assert_eq!(super::super::delegation::MAX_DELEGATION_USDC, 500_000_000);
    }

    #[test]
    fn test_build_payout_transfer_is_signed() {
        let (json, kp) = test_keypair_json();
        let svc = DelegationService::from_json_keypair(&json).unwrap();

        let vault_ata = Pubkey::new_unique();
        let user_ata = Pubkey::new_unique();
        let blockhash = solana_sdk::hash::Hash::new_unique();

        let tx = svc.build_payout_transfer(&vault_ata, &user_ata, 50_000_000, blockhash).unwrap();

        // Platform signer is fee payer (first key)
        assert_eq!(tx.message.account_keys[0], kp.pubkey());
        // Signed
        assert_eq!(tx.signatures.len(), 1);
        assert_ne!(tx.signatures[0], solana_sdk::signature::Signature::default());

        // Transfer instruction: source = vault, dest = user
        let ix = &tx.message.instructions[0];
        assert_eq!(ix.data[0], 3u8); // Transfer tag
        let amount = u64::from_le_bytes(ix.data[1..9].try_into().unwrap());
        assert_eq!(amount, 50_000_000);

        let keys = &tx.message.account_keys;
        let source_idx = ix.accounts[0] as usize;
        let dest_idx = ix.accounts[1] as usize;
        let authority_idx = ix.accounts[2] as usize;
        assert_eq!(keys[source_idx], vault_ata);
        assert_eq!(keys[dest_idx], user_ata);
        assert_eq!(keys[authority_idx], kp.pubkey()); // owner authority
    }
}
