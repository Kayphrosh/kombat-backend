module kombat::wager {

use std::string::{Self, String};
use sui::balance::{Self, Balance};
use sui::clock::{Self, Clock};
use sui::coin::{Self, Coin};
use sui::event;
use sui::object::{Self, UID};
use sui::transfer;
use sui::tx_context::{Self, TxContext};

const STATUS_OPEN: u8 = 0;
const STATUS_ACTIVE: u8 = 1;
const STATUS_RESOLVED: u8 = 2;
const STATUS_CANCELLED: u8 = 3;

/// Sentinel meaning "no specific address" (open challenger / unresolved winner).
const NO_ADDRESS: address = @0x0;

const E_NOT_OPEN: u64 = 1;
const E_NOT_ACTIVE: u64 = 2;
const E_ZERO_AMOUNT: u64 = 3;
const E_STAKE_MISMATCH: u64 = 4;
const E_EXPIRED: u64 = 5;
const E_NOT_RESOLVER: u64 = 6;
const E_NOT_INITIATOR: u64 = 7;
const E_INVALID_WINNER: u64 = 8;
const E_NOT_TARGETED_CHALLENGER: u64 = 9;
const E_SELF_ACCEPT: u64 = 10;
const E_BALANCE_LOW: u64 = 11;

/// A 1-v-1 wager. Both participants' stakes are escrowed in `vault`; the
/// resolver declares the winner, who receives the whole vault.
public struct Wager<phantom CoinType> has key {
    id: UID,
    initiator: address,
    /// `NO_ADDRESS` until accepted, or a targeted opponent set at creation.
    challenger: address,
    /// Trusted party (e.g. backend admin) authorized to resolve.
    resolver: address,
    description: String,
    initiator_option: String,
    stake_amount: u64,
    status: u8,
    /// `NO_ADDRESS` until resolved.
    winner: address,
    expiry_ms: u64,
    vault: Balance<CoinType>,
    created_at_ms: u64,
    resolved_at_ms: u64,
}

public struct WagerCreated has copy, drop {
    wager_id: address,
    initiator: address,
    challenger: address,
    resolver: address,
    stake_amount: u64,
    expiry_ms: u64,
    created_at_ms: u64,
}

public struct WagerAccepted has copy, drop {
    wager_id: address,
    challenger: address,
    total_escrow: u64,
    accepted_at_ms: u64,
}

public struct WagerResolved has copy, drop {
    wager_id: address,
    winner: address,
    payout_amount: u64,
    resolved_at_ms: u64,
}

public struct WagerCancelled has copy, drop {
    wager_id: address,
    initiator: address,
    refund_amount: u64,
    cancelled_at_ms: u64,
}

/// Create a wager and escrow the initiator's stake. Pass `challenger = @0x0`
/// for an open wager, or a specific address to target an opponent.
public entry fun create_wager<CoinType>(
    stake: Coin<CoinType>,
    description: vector<u8>,
    initiator_option: vector<u8>,
    challenger: address,
    expiry_ms: u64,
    resolver: address,
    clock: &Clock,
    ctx: &mut TxContext,
) {
    let now_ms = clock::timestamp_ms(clock);
    let initiator = tx_context::sender(ctx);
    let stake_amount = coin::value(&stake);
    assert!(stake_amount > 0, E_ZERO_AMOUNT);

    let wager = Wager<CoinType> {
        id: object::new(ctx),
        initiator,
        challenger,
        resolver,
        description: string::utf8(description),
        initiator_option: string::utf8(initiator_option),
        stake_amount,
        status: STATUS_OPEN,
        winner: NO_ADDRESS,
        expiry_ms,
        vault: coin::into_balance(stake),
        created_at_ms: now_ms,
        resolved_at_ms: 0,
    };

    event::emit(WagerCreated {
        wager_id: object::uid_to_address(&wager.id),
        initiator,
        challenger,
        resolver,
        stake_amount,
        expiry_ms,
        created_at_ms: now_ms,
    });

    transfer::share_object(wager);
}

/// Challenger matches the stake and activates the wager.
public entry fun accept_wager<CoinType>(
    wager: &mut Wager<CoinType>,
    payment: Coin<CoinType>,
    clock: &Clock,
    ctx: &mut TxContext,
) {
    let now_ms = clock::timestamp_ms(clock);
    let sender = tx_context::sender(ctx);

    assert!(wager.status == STATUS_OPEN, E_NOT_OPEN);
    assert!(now_ms <= wager.expiry_ms, E_EXPIRED);
    assert!(sender != wager.initiator, E_SELF_ACCEPT);
    // If the wager targeted a specific challenger, only they may accept.
    if (wager.challenger != NO_ADDRESS) {
        assert!(sender == wager.challenger, E_NOT_TARGETED_CHALLENGER);
    };
    assert!(coin::value(&payment) == wager.stake_amount, E_STAKE_MISMATCH);

    balance::join(&mut wager.vault, coin::into_balance(payment));
    wager.challenger = sender;
    wager.status = STATUS_ACTIVE;

    event::emit(WagerAccepted {
        wager_id: object::uid_to_address(&wager.id),
        challenger: sender,
        total_escrow: balance::value(&wager.vault),
        accepted_at_ms: now_ms,
    });
}

/// Resolver declares the winner, who receives the entire escrow.
public entry fun resolve_wager<CoinType>(
    wager: &mut Wager<CoinType>,
    winner: address,
    clock: &Clock,
    ctx: &mut TxContext,
) {
    let now_ms = clock::timestamp_ms(clock);
    assert!(tx_context::sender(ctx) == wager.resolver, E_NOT_RESOLVER);
    assert!(wager.status == STATUS_ACTIVE, E_NOT_ACTIVE);
    assert!(winner == wager.initiator || winner == wager.challenger, E_INVALID_WINNER);

    let payout = balance::value(&wager.vault);
    assert!(balance::value(&wager.vault) >= payout, E_BALANCE_LOW);

    wager.status = STATUS_RESOLVED;
    wager.winner = winner;
    wager.resolved_at_ms = now_ms;

    let coin = coin::from_balance(balance::split(&mut wager.vault, payout), ctx);
    transfer::public_transfer(coin, winner);

    event::emit(WagerResolved {
        wager_id: object::uid_to_address(&wager.id),
        winner,
        payout_amount: payout,
        resolved_at_ms: now_ms,
    });
}

/// Initiator cancels an unaccepted wager and reclaims their stake.
public entry fun cancel_wager<CoinType>(
    wager: &mut Wager<CoinType>,
    clock: &Clock,
    ctx: &mut TxContext,
) {
    let now_ms = clock::timestamp_ms(clock);
    assert!(tx_context::sender(ctx) == wager.initiator, E_NOT_INITIATOR);
    assert!(wager.status == STATUS_OPEN, E_NOT_OPEN);

    let refund = balance::value(&wager.vault);
    wager.status = STATUS_CANCELLED;
    wager.resolved_at_ms = now_ms;

    let coin = coin::from_balance(balance::split(&mut wager.vault, refund), ctx);
    transfer::public_transfer(coin, wager.initiator);

    event::emit(WagerCancelled {
        wager_id: object::uid_to_address(&wager.id),
        initiator: wager.initiator,
        refund_amount: refund,
        cancelled_at_ms: now_ms,
    });
}

// ── Views ──────────────────────────────────────────────────────────────────

public fun status<CoinType>(wager: &Wager<CoinType>): u8 {
    wager.status
}

public fun stake_amount<CoinType>(wager: &Wager<CoinType>): u64 {
    wager.stake_amount
}

public fun participants<CoinType>(wager: &Wager<CoinType>): (address, address) {
    (wager.initiator, wager.challenger)
}

public fun winner<CoinType>(wager: &Wager<CoinType>): address {
    wager.winner
}
}
