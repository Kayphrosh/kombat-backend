use anchor_lang::prelude::*;

#[event]
pub struct WagerCreated {
    pub wager: Pubkey, // PDA
    pub wager_id: u64,
    pub initiator: Pubkey,
    pub stake_lamports: u64,
    pub expiry_ts: i64,
}

#[event]
pub struct WagerAccepted {
    pub wager: Pubkey,
    pub wager_id: u64,
    pub challenger: Pubkey,
}

#[event]
pub struct WagerResolved {
    pub wager: Pubkey,
    pub wager_id: u64,
    pub winner: Pubkey,
    pub resolver: Pubkey,
}

#[event]
pub struct WagerCancelled {
    pub wager: Pubkey,
    pub wager_id: u64,
}

#[event]
pub struct WagerDisputed {
    pub wager: Pubkey,
    pub wager_id: u64,
    pub opener: Pubkey,
}
