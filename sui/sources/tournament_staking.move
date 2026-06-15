module kombat::tournament_staking {

use std::string::{Self, String};
use sui::balance::{Self, Balance};
use sui::clock::{Self, Clock};
use sui::coin::{Self, Coin};
use sui::event;
use sui::object::{Self, UID};
use sui::transfer;
use sui::tx_context::{Self, TxContext};

const STATUS_OPEN: u8 = 0;
const STATUS_RESOLVED: u8 = 1;
const STATUS_CANCELLED: u8 = 2;

const OUTCOME_A: u8 = 1;
const OUTCOME_B: u8 = 2;

const E_NOT_ADMIN: u64 = 1;
const E_POOL_NOT_OPEN: u64 = 2;
const E_POOL_NOT_CLOSED: u64 = 3;
const E_INVALID_OUTCOME: u64 = 4;
const E_ZERO_AMOUNT: u64 = 5;
const E_MATCH_MISMATCH: u64 = 6;
const E_NOT_WINNER: u64 = 7;
const E_NOT_REFUNDABLE: u64 = 8;
const E_STAKING_CLOSED: u64 = 9;
const E_POOL_BALANCE_LOW: u64 = 10;
const E_NOT_RECEIPT_OWNER: u64 = 11;
const E_INVALID_ASK: u64 = 12;
const E_LISTING_EXPIRED: u64 = 13;
const E_PAYMENT_MISMATCH: u64 = 14;
const E_NOT_SELLER: u64 = 15;

public struct AdminCap has key {
    id: UID,
}

public struct TournamentPool<phantom CoinType> has key {
    id: UID,
    admin: address,
    match_id: String,
    outcome_a: String,
    outcome_b: String,
    status: u8,
    winning_outcome: u8,
    stake_deadline_ms: u64,
    total_a: u64,
    total_b: u64,
    vault: Balance<CoinType>,
    created_at_ms: u64,
    resolved_at_ms: u64,
}

public struct StakeReceipt<phantom CoinType> has key, store {
    id: UID,
    pool_id: address,
    owner: address,
    match_id: String,
    outcome: u8,
    amount: u64,
    odds_numerator_at_stake: u64,
    odds_denominator_at_stake: u64,
    created_at_ms: u64,
}

public struct ReceiptListing<phantom CoinType> has key {
    id: UID,
    seller: address,
    receipt_id: address,
    pool_id: address,
    match_id: String,
    outcome: u8,
    amount: u64,
    ask_amount: u64,
    listed_at_ms: u64,
    expires_at_ms: u64,
    receipt: StakeReceipt<CoinType>,
}

public struct PoolCreated has copy, drop {
    pool_id: address,
    admin: address,
    match_id: String,
    outcome_a: String,
    outcome_b: String,
    stake_deadline_ms: u64,
    created_at_ms: u64,
}

public struct StakePlaced has copy, drop {
    pool_id: address,
    receipt_id: address,
    owner: address,
    match_id: String,
    outcome: u8,
    amount: u64,
    total_a: u64,
    total_b: u64,
    created_at_ms: u64,
}

public struct PoolResolved has copy, drop {
    pool_id: address,
    match_id: String,
    winning_outcome: u8,
    total_a: u64,
    total_b: u64,
    resolved_at_ms: u64,
}

public struct PoolCancelled has copy, drop {
    pool_id: address,
    match_id: String,
    cancelled_at_ms: u64,
}

public struct StakeClaimed has copy, drop {
    pool_id: address,
    receipt_id: address,
    owner: address,
    match_id: String,
    outcome: u8,
    staked_amount: u64,
    payout_amount: u64,
    claimed_at_ms: u64,
}

public struct ReceiptListed has copy, drop {
    listing_id: address,
    receipt_id: address,
    seller: address,
    pool_id: address,
    match_id: String,
    outcome: u8,
    amount: u64,
    ask_amount: u64,
    listed_at_ms: u64,
    expires_at_ms: u64,
}

public struct ReceiptSold has copy, drop {
    listing_id: address,
    receipt_id: address,
    seller: address,
    buyer: address,
    pool_id: address,
    match_id: String,
    outcome: u8,
    amount: u64,
    sale_amount: u64,
    sold_at_ms: u64,
}

public struct ReceiptListingCancelled has copy, drop {
    listing_id: address,
    receipt_id: address,
    seller: address,
    cancelled_at_ms: u64,
}

fun init(ctx: &mut TxContext) {
    transfer::transfer(
        AdminCap { id: object::new(ctx) },
        tx_context::sender(ctx),
    );
}

public entry fun create_pool<CoinType>(
    _: &AdminCap,
    match_id: vector<u8>,
    outcome_a: vector<u8>,
    outcome_b: vector<u8>,
    stake_deadline_ms: u64,
    clock: &Clock,
    ctx: &mut TxContext,
) {
    let now_ms = clock::timestamp_ms(clock);
    let admin = tx_context::sender(ctx);
    let match_id = string::utf8(match_id);
    let outcome_a = string::utf8(outcome_a);
    let outcome_b = string::utf8(outcome_b);

    let pool = TournamentPool<CoinType> {
        id: object::new(ctx),
        admin,
        match_id,
        outcome_a,
        outcome_b,
        status: STATUS_OPEN,
        winning_outcome: 0,
        stake_deadline_ms,
        total_a: 0,
        total_b: 0,
        vault: balance::zero<CoinType>(),
        created_at_ms: now_ms,
        resolved_at_ms: 0,
    };

    event::emit(PoolCreated {
        pool_id: object::uid_to_address(&pool.id),
        admin,
        match_id: pool.match_id,
        outcome_a: pool.outcome_a,
        outcome_b: pool.outcome_b,
        stake_deadline_ms,
        created_at_ms: now_ms,
    });

    transfer::share_object(pool);
}

public entry fun stake<CoinType>(
    pool: &mut TournamentPool<CoinType>,
    outcome: u8,
    payment: Coin<CoinType>,
    clock: &Clock,
    ctx: &mut TxContext,
) {
    let now_ms = clock::timestamp_ms(clock);
    assert!(pool.status == STATUS_OPEN, E_POOL_NOT_OPEN);
    assert!(now_ms <= pool.stake_deadline_ms, E_STAKING_CLOSED);
    assert!(outcome == OUTCOME_A || outcome == OUTCOME_B, E_INVALID_OUTCOME);

    let amount = coin::value(&payment);
    assert!(amount > 0, E_ZERO_AMOUNT);

    let total_before = pool.total_a + pool.total_b;
    let side_before = if (outcome == OUTCOME_A) pool.total_a else pool.total_b;
    let odds_numerator = total_before + amount;
    let odds_denominator = side_before + amount;

    balance::join(&mut pool.vault, coin::into_balance(payment));
    if (outcome == OUTCOME_A) {
        pool.total_a = pool.total_a + amount;
    } else {
        pool.total_b = pool.total_b + amount;
    };

    let receipt = StakeReceipt<CoinType> {
        id: object::new(ctx),
        pool_id: object::uid_to_address(&pool.id),
        owner: tx_context::sender(ctx),
        match_id: pool.match_id,
        outcome,
        amount,
        odds_numerator_at_stake: odds_numerator,
        odds_denominator_at_stake: odds_denominator,
        created_at_ms: now_ms,
    };
    let receipt_id = object::uid_to_address(&receipt.id);
    let owner = receipt.owner;

    event::emit(StakePlaced {
        pool_id: object::uid_to_address(&pool.id),
        receipt_id,
        owner,
        match_id: receipt.match_id,
        outcome,
        amount,
        total_a: pool.total_a,
        total_b: pool.total_b,
        created_at_ms: now_ms,
    });

    transfer::transfer(receipt, owner);
}

public entry fun resolve<CoinType>(
    cap: &AdminCap,
    pool: &mut TournamentPool<CoinType>,
    winning_outcome: u8,
    clock: &Clock,
    ctx: &mut TxContext,
) {
    assert_admin(cap, pool, ctx);
    assert!(pool.status == STATUS_OPEN, E_POOL_NOT_OPEN);
    assert!(
        winning_outcome == OUTCOME_A || winning_outcome == OUTCOME_B,
        E_INVALID_OUTCOME,
    );

    let now_ms = clock::timestamp_ms(clock);
    pool.status = STATUS_RESOLVED;
    pool.winning_outcome = winning_outcome;
    pool.resolved_at_ms = now_ms;

    event::emit(PoolResolved {
        pool_id: object::uid_to_address(&pool.id),
        match_id: pool.match_id,
        winning_outcome,
        total_a: pool.total_a,
        total_b: pool.total_b,
        resolved_at_ms: now_ms,
    });
}

public entry fun cancel<CoinType>(
    cap: &AdminCap,
    pool: &mut TournamentPool<CoinType>,
    clock: &Clock,
    ctx: &mut TxContext,
) {
    assert_admin(cap, pool, ctx);
    assert!(pool.status == STATUS_OPEN, E_POOL_NOT_OPEN);

    let now_ms = clock::timestamp_ms(clock);
    pool.status = STATUS_CANCELLED;
    pool.resolved_at_ms = now_ms;

    event::emit(PoolCancelled {
        pool_id: object::uid_to_address(&pool.id),
        match_id: pool.match_id,
        cancelled_at_ms: now_ms,
    });
}

public entry fun claim<CoinType>(
    pool: &mut TournamentPool<CoinType>,
    receipt: StakeReceipt<CoinType>,
    clock: &Clock,
    ctx: &mut TxContext,
) {
    assert!(pool.status == STATUS_RESOLVED, E_POOL_NOT_CLOSED);
    assert!(receipt.pool_id == object::uid_to_address(&pool.id), E_MATCH_MISMATCH);
    assert!(receipt.outcome == pool.winning_outcome, E_NOT_WINNER);
    let receipt_id = object::uid_to_address(&receipt.id);

    let winning_total = if (pool.winning_outcome == OUTCOME_A) pool.total_a else pool.total_b;
    let total_pool = pool.total_a + pool.total_b;
    let payout = if (winning_total == 0) 0 else {
        ((receipt.amount as u128) * (total_pool as u128) / (winning_total as u128)) as u64
    };
    assert!(balance::value(&pool.vault) >= payout, E_POOL_BALANCE_LOW);

    let StakeReceipt {
        id,
        pool_id: _,
        owner,
        match_id,
        outcome,
        amount,
        odds_numerator_at_stake: _,
        odds_denominator_at_stake: _,
        created_at_ms: _,
    } = receipt;
    object::delete(id);

    let coin = coin::from_balance(balance::split(&mut pool.vault, payout), ctx);
    transfer::public_transfer(coin, owner);

    event::emit(StakeClaimed {
        pool_id: object::uid_to_address(&pool.id),
        receipt_id,
        owner,
        match_id,
        outcome,
        staked_amount: amount,
        payout_amount: payout,
        claimed_at_ms: clock::timestamp_ms(clock),
    });
}

public entry fun refund<CoinType>(
    pool: &mut TournamentPool<CoinType>,
    receipt: StakeReceipt<CoinType>,
    clock: &Clock,
    ctx: &mut TxContext,
) {
    assert!(
        pool.status == STATUS_CANCELLED || is_one_sided_resolved_pool(pool),
        E_NOT_REFUNDABLE,
    );
    assert!(receipt.pool_id == object::uid_to_address(&pool.id), E_MATCH_MISMATCH);
    let receipt_id = object::uid_to_address(&receipt.id);

    let StakeReceipt {
        id,
        pool_id: _,
        owner,
        match_id,
        outcome,
        amount,
        odds_numerator_at_stake: _,
        odds_denominator_at_stake: _,
        created_at_ms: _,
    } = receipt;
    object::delete(id);

    assert!(balance::value(&pool.vault) >= amount, E_POOL_BALANCE_LOW);
    let coin = coin::from_balance(balance::split(&mut pool.vault, amount), ctx);
    transfer::public_transfer(coin, owner);

    event::emit(StakeClaimed {
        pool_id: object::uid_to_address(&pool.id),
        receipt_id,
        owner,
        match_id,
        outcome,
        staked_amount: amount,
        payout_amount: amount,
        claimed_at_ms: clock::timestamp_ms(clock),
    });
}

public entry fun list_receipt<CoinType>(
    receipt: StakeReceipt<CoinType>,
    ask_amount: u64,
    expires_at_ms: u64,
    clock: &Clock,
    ctx: &mut TxContext,
) {
    let now_ms = clock::timestamp_ms(clock);
    let seller = tx_context::sender(ctx);
    assert!(receipt.owner == seller, E_NOT_RECEIPT_OWNER);
    assert!(ask_amount > 0, E_INVALID_ASK);
    assert!(expires_at_ms > now_ms, E_LISTING_EXPIRED);

    let receipt_id = object::uid_to_address(&receipt.id);
    let pool_id = receipt.pool_id;
    let match_id = receipt.match_id;
    let outcome = receipt.outcome;
    let amount = receipt.amount;

    let listing = ReceiptListing<CoinType> {
        id: object::new(ctx),
        seller,
        receipt_id,
        pool_id,
        match_id,
        outcome,
        amount,
        ask_amount,
        listed_at_ms: now_ms,
        expires_at_ms,
        receipt,
    };

    event::emit(ReceiptListed {
        listing_id: object::uid_to_address(&listing.id),
        receipt_id,
        seller,
        pool_id,
        match_id: listing.match_id,
        outcome,
        amount,
        ask_amount,
        listed_at_ms: now_ms,
        expires_at_ms,
    });

    transfer::share_object(listing);
}

public entry fun buy_receipt<CoinType>(
    listing: ReceiptListing<CoinType>,
    payment: Coin<CoinType>,
    clock: &Clock,
    ctx: &mut TxContext,
) {
    let now_ms = clock::timestamp_ms(clock);
    assert!(now_ms <= listing.expires_at_ms, E_LISTING_EXPIRED);
    assert!(coin::value(&payment) == listing.ask_amount, E_PAYMENT_MISMATCH);

    let buyer = tx_context::sender(ctx);
    let ReceiptListing {
        id,
        seller,
        receipt_id,
        pool_id,
        match_id,
        outcome,
        amount,
        ask_amount,
        listed_at_ms: _,
        expires_at_ms: _,
        receipt,
    } = listing;
    let listing_id = object::uid_to_address(&id);
    object::delete(id);

    let mut receipt = receipt;
    receipt.owner = buyer;

    transfer::public_transfer(payment, seller);
    transfer::transfer(receipt, buyer);

    event::emit(ReceiptSold {
        listing_id,
        receipt_id,
        seller,
        buyer,
        pool_id,
        match_id,
        outcome,
        amount,
        sale_amount: ask_amount,
        sold_at_ms: now_ms,
    });
}

public entry fun cancel_receipt_listing<CoinType>(
    listing: ReceiptListing<CoinType>,
    clock: &Clock,
    ctx: &mut TxContext,
) {
    let sender = tx_context::sender(ctx);
    assert!(sender == listing.seller, E_NOT_SELLER);

    let ReceiptListing {
        id,
        seller,
        receipt_id,
        pool_id: _,
        match_id: _,
        outcome: _,
        amount: _,
        ask_amount: _,
        listed_at_ms: _,
        expires_at_ms: _,
        receipt,
    } = listing;
    let listing_id = object::uid_to_address(&id);
    object::delete(id);
    transfer::transfer(receipt, seller);

    event::emit(ReceiptListingCancelled {
        listing_id,
        receipt_id,
        seller,
        cancelled_at_ms: clock::timestamp_ms(clock),
    });
}

public fun pool_status<CoinType>(pool: &TournamentPool<CoinType>): u8 {
    pool.status
}

public fun pool_totals<CoinType>(pool: &TournamentPool<CoinType>): (u64, u64, u64) {
    (pool.total_a, pool.total_b, pool.total_a + pool.total_b)
}

public fun receipt_amount<CoinType>(receipt: &StakeReceipt<CoinType>): u64 {
    receipt.amount
}

fun assert_admin<CoinType>(
    _: &AdminCap,
    pool: &TournamentPool<CoinType>,
    ctx: &TxContext,
) {
    assert!(tx_context::sender(ctx) == pool.admin, E_NOT_ADMIN);
}

fun is_one_sided_resolved_pool<CoinType>(pool: &TournamentPool<CoinType>): bool {
    pool.status == STATUS_RESOLVED && (pool.total_a == 0 || pool.total_b == 0)
}
}
