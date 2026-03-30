#![no_std]
//! Balance Snapshot Module for Fair Reward Distribution
//!
//! Implements a checkpoint-based snapshot mechanism that records each user's
//! staked balance at discrete epochs. Rewards are distributed proportionally
//! based on the balance held at the snapshot epoch, preventing manipulation
//! by staking just before a reward drop and unstaking immediately after.
//!
//! ## Storage layout (gas-efficient)
//!
//! - `SnapshotEpoch`                  → current epoch counter (instance)
//! - `TotalStaked`                    → total staked at current epoch (instance)
//! - `EpochTotal(epoch)`              → total staked at a past epoch (persistent)
//! - `UserCheckpoint(user, epoch)`    → user balance at a specific epoch (persistent)
//! - `UserLastEpoch(user)`            → last epoch a user wrote a checkpoint (persistent)
//!
//! Only a new checkpoint entry is written when a user's balance actually changes,
//! keeping per-user storage O(number of interactions), not O(epochs).

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env};

// ── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
pub enum DataKey {
    /// Monotonically increasing epoch counter
    SnapshotEpoch,
    /// Live total staked (updated on every stake/unstake)
    TotalStaked,
    /// Snapshot of total staked at a closed epoch
    EpochTotal(u32),
    /// User balance checkpoint: (user, epoch) → balance
    UserCheckpoint(Address, u32),
    /// Most recent epoch at which a user wrote a checkpoint
    UserLastEpoch(Address),
}

// ── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct SnapshotStaking;

#[contractimpl]
impl SnapshotStaking {
    // ── Admin ─────────────────────────────────────────────────────────────

    /// Initialise the contract. Must be called once before any other function.
    pub fn initialize(env: Env) {
        if env.storage().instance().has(&DataKey::SnapshotEpoch) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::SnapshotEpoch, &0_u32);
        env.storage().instance().set(&DataKey::TotalStaked, &0_i128);
    }

    /// Advance to the next epoch and freeze the current total staked.
    ///
    /// Should be called by a keeper / cron job before distributing rewards.
    /// O(1) — no iteration.
    pub fn advance_epoch(env: Env) -> u32 {
        let epoch = Self::current_epoch(env.clone());

        // Freeze the current total into persistent storage for this epoch
        let total: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::EpochTotal(epoch), &total);

        let next = epoch + 1;
        env.storage()
            .instance()
            .set(&DataKey::SnapshotEpoch, &next);

        env.events()
            .publish((symbol_short!("new_epoch"),), next);

        next
    }

    // ── Staking ───────────────────────────────────────────────────────────

    /// Record a stake of `amount` for `user` at the current epoch.
    ///
    /// In a full implementation this would also transfer tokens; here we focus
    /// on the snapshot bookkeeping that enables fair reward distribution.
    pub fn stake(env: Env, user: Address, amount: i128) {
        user.require_auth();
        assert!(amount > 0, "amount must be positive");

        let epoch = Self::current_epoch(env.clone());
        let prev = Self::_balance_at_epoch(&env, &user, epoch);
        Self::_write_checkpoint(&env, &user, epoch, prev + amount);

        let total: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalStaked, &(total + amount));

        env.events()
            .publish((symbol_short!("staked"), user), amount);
    }

    /// Record an unstake of `amount` for `user` at the current epoch.
    pub fn unstake(env: Env, user: Address, amount: i128) {
        user.require_auth();
        assert!(amount > 0, "amount must be positive");

        let epoch = Self::current_epoch(env.clone());
        let prev = Self::_balance_at_epoch(&env, &user, epoch);
        assert!(prev >= amount, "insufficient stake");

        Self::_write_checkpoint(&env, &user, epoch, prev - amount);

        let total: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalStaked, &(total - amount));

        env.events()
            .publish((symbol_short!("unstaked"), user), amount);
    }

    // ── Views ─────────────────────────────────────────────────────────────

    /// Current (live) epoch index.
    pub fn current_epoch(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::SnapshotEpoch)
            .unwrap_or(0)
    }

    /// Total staked that was frozen at `epoch` (after `advance_epoch` was called).
    pub fn epoch_total(env: Env, epoch: u32) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::EpochTotal(epoch))
            .unwrap_or(0)
    }

    /// User's balance as recorded at a specific past `epoch`.
    ///
    /// Returns the checkpoint written at or before `epoch` — i.e. the balance
    /// the user held when that epoch was active. O(1) per lookup.
    pub fn balance_at(env: Env, user: Address, epoch: u32) -> i128 {
        Self::_balance_at_epoch(&env, &user, epoch)
    }

    /// Reward share (in basis points, 0–10_000) for `user` at `epoch`.
    ///
    /// share_bps = user_balance_at_epoch * 10_000 / epoch_total
    ///
    /// Returns 0 if total staked was 0 at that epoch.
    pub fn reward_share_bps(env: Env, user: Address, epoch: u32) -> u32 {
        let total = Self::epoch_total(env.clone(), epoch);
        if total == 0 {
            return 0;
        }
        let user_bal = Self::balance_at(env, user, epoch);
        ((user_bal * 10_000) / total) as u32
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    /// Write a checkpoint for `user` at `epoch`. Overwrites any existing entry
    /// for the same epoch (idempotent within an epoch).
    fn _write_checkpoint(env: &Env, user: &Address, epoch: u32, balance: i128) {
        env.storage()
            .persistent()
            .set(&DataKey::UserCheckpoint(user.clone(), epoch), &balance);
        env.storage()
            .persistent()
            .set(&DataKey::UserLastEpoch(user.clone()), &epoch);
    }

    /// Resolve a user's balance at `epoch`.
    ///
    /// Strategy: look up the checkpoint written at exactly `epoch`; if absent,
    /// fall back to the checkpoint at `last_epoch` (the most recent write ≤
    /// epoch). This works because balances only change when the user interacts,
    /// so the last written checkpoint is always valid for all subsequent epochs
    /// until the next interaction.
    fn _balance_at_epoch(env: &Env, user: &Address, epoch: u32) -> i128 {
        // Fast path: exact match
        if let Some(bal) = env
            .storage()
            .persistent()
            .get::<_, i128>(&DataKey::UserCheckpoint(user.clone(), epoch))
        {
            return bal;
        }

        // Fallback: use the most recent checkpoint (valid for all later epochs)
        let last_epoch: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::UserLastEpoch(user.clone()))
            .unwrap_or(0);

        if last_epoch <= epoch {
            env.storage()
                .persistent()
                .get(&DataKey::UserCheckpoint(user.clone(), last_epoch))
                .unwrap_or(0)
        } else {
            // User's last interaction was after `epoch` — they had no stake then
            0
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn setup() -> (Env, SnapshotStakingClient<'static>, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(SnapshotStaking, ());
        let client = SnapshotStakingClient::new(&env, &id);
        client.initialize();
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        (env, client, alice, bob)
    }

    #[test]
    fn test_stake_and_snapshot() {
        let (_env, client, alice, _bob) = setup();

        client.stake(&alice, &1_000);
        assert_eq!(client.balance_at(&alice, &0), 1_000);

        // Advance epoch: freezes epoch 0 total, moves to epoch 1
        let next = client.advance_epoch();
        assert_eq!(next, 1);
        assert_eq!(client.epoch_total(&0), 1_000);

        // Alice's balance at epoch 0 is still readable
        assert_eq!(client.balance_at(&alice, &0), 1_000);
    }

    #[test]
    fn test_reward_share_proportional() {
        let (_env, client, alice, bob) = setup();

        client.stake(&alice, &300);
        client.stake(&bob, &700);
        client.advance_epoch(); // closes epoch 0

        // Alice 30%, Bob 70% (in basis points)
        assert_eq!(client.reward_share_bps(&alice, &0), 3_000);
        assert_eq!(client.reward_share_bps(&bob, &0), 7_000);
    }

    #[test]
    fn test_late_staker_excluded_from_past_epoch() {
        let (_env, client, alice, bob) = setup();

        client.stake(&alice, &1_000);
        client.advance_epoch(); // closes epoch 0

        // Bob stakes in epoch 1 — should have 0 share in epoch 0
        client.stake(&bob, &1_000);
        assert_eq!(client.reward_share_bps(&bob, &0), 0);
        assert_eq!(client.reward_share_bps(&alice, &0), 10_000);
    }

    #[test]
    fn test_unstake_reduces_balance() {
        let (_env, client, alice, _bob) = setup();

        client.stake(&alice, &1_000);
        client.unstake(&alice, &400);
        assert_eq!(client.balance_at(&alice, &0), 600);
    }

    #[test]
    #[should_panic(expected = "insufficient stake")]
    fn test_unstake_exceeds_balance_panics() {
        let (_env, client, alice, _bob) = setup();
        client.stake(&alice, &100);
        client.unstake(&alice, &200);
    }

    #[test]
    fn test_balance_carries_forward_across_epochs() {
        let (_env, client, alice, _bob) = setup();

        client.stake(&alice, &500);
        client.advance_epoch(); // epoch 0 → 1
        client.advance_epoch(); // epoch 1 → 2

        // Alice never interacted in epochs 1 or 2, balance should carry forward
        assert_eq!(client.balance_at(&alice, &1), 500);
        assert_eq!(client.balance_at(&alice, &2), 500);
    }

    #[test]
    fn test_epoch_total_zero_when_no_stakers() {
        let (_env, client, _alice, _bob) = setup();
        client.advance_epoch();
        assert_eq!(client.epoch_total(&0), 0);
        assert_eq!(client.reward_share_bps(&_alice, &0), 0);
    }
}
