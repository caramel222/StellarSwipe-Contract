#![no_std]

mod errors;
pub mod keeper;
pub mod risk_gates;
pub mod sdex;
pub mod triggers;
pub mod wire;

use errors::{ContractError, InsufficientBalanceDetail};
use risk_gates::{check_position_limit, check_user_balance, DEFAULT_ESTIMATED_COPY_TRADE_FEE};
use sdex::{execute_sdex_swap, min_received_from_slippage};
use wire::{TradeOrder, TradeStatus, TRADE_TIMEOUT_LEDGERS};

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, IntoVal, Symbol, Val, Vec};

use keeper::{TriggerablePosition, compute_keeper_reward, get_triggerable_positions, register_watch};

/// Instance storage keys.
#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    Admin,
    /// Contract implementing `get_open_position_count(user) -> u32` (UserPortfolio).
    UserPortfolio,
    /// When set to `true`, this user bypasses [`risk_gates::MAX_POSITIONS_PER_USER`].
    PositionLimitExempt(Address),
    /// Pending limit order by trade_id.
    TradeOrder(u64),
    /// Oracle contract used by stop-loss/take-profit triggers.
    Oracle,
    /// Portfolio contract used by stop-loss/take-profit close calls.
    StopLossPortfolio,
    /// Overrides default estimated fee used in balance checks.
    CopyTradeEstimatedFee,
    /// Last balance shortfall for `user`.
    LastInsufficientBalance(Address),
    /// SDEX router contract address.
    SdexRouter,
    /// SAC token used to pay keeper rewards.
    KeeperRewardToken,
    /// Accumulated reward balance owed to a keeper address.
    KeeperReward(Address),
}

/// Symbol invoked on the portfolio after a successful limit check.
pub const RECORD_COPY_POSITION_FN: &str = "record_copy_position";

/// Temporary-storage key for the reentrancy lock on `execute_copy_trade`.
const EXECUTION_LOCK: &str = "ExecLock";

#[contract]
pub struct TradeExecutorContract;

fn effective_estimated_fee(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&StorageKey::CopyTradeEstimatedFee)
        .unwrap_or(DEFAULT_ESTIMATED_COPY_TRADE_FEE)
}

#[contractimpl]
impl TradeExecutorContract {
    /// One-time init; stores admin.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
    }

    // ── Portfolio configuration ───────────────────────────────────────────────

    pub fn set_user_portfolio(env: Env, portfolio: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::UserPortfolio, &portfolio);
    }

    pub fn get_user_portfolio(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::UserPortfolio)
    }

    // ── Fee configuration ─────────────────────────────────────────────────────

    pub fn set_copy_trade_estimated_fee(env: Env, fee: i128) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if fee < 0 {
            panic!("fee must be non-negative");
        }
        env.storage()
            .instance()
            .set(&StorageKey::CopyTradeEstimatedFee, &fee);
    }

    pub fn get_copy_trade_estimated_fee(env: Env) -> i128 {
        effective_estimated_fee(&env)
    }

    // ── Position limit exemption ──────────────────────────────────────────────

    pub fn set_position_limit_exempt(env: Env, user: Address, exempt: bool) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        let key = StorageKey::PositionLimitExempt(user);
        if exempt {
            env.storage().instance().set(&key, &true);
        } else {
            env.storage().instance().remove(&key);
        }
    }

    pub fn is_position_limit_exempt(env: Env, user: Address) -> bool {
        let key = StorageKey::PositionLimitExempt(user);
        env.storage().instance().get(&key).unwrap_or(false)
    }

    // ── Stop-loss / take-profit configuration ─────────────────────────────────

    pub fn set_oracle(env: Env, oracle: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&Symbol::new(&env, triggers::ORACLE_KEY), &oracle);
    }

    pub fn set_stop_loss_portfolio(env: Env, portfolio: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&Symbol::new(&env, triggers::PORTFOLIO_KEY), &portfolio);
    }

    pub fn set_stop_loss_price(env: Env, user: Address, trade_id: u64, stop_loss_price: i128) {
        user.require_auth();
        triggers::set_stop_loss(&env, &user, trade_id, stop_loss_price);
    }

    pub fn set_stop_loss_price_with_pair(
        env: Env,
        user: Address,
        trade_id: u64,
        stop_loss_price: i128,
        asset_pair: u32,
    ) {
        user.require_auth();
        triggers::set_stop_loss(&env, &user, trade_id, stop_loss_price);
        register_watch(&env, &user, trade_id, asset_pair);
    }

    pub fn check_and_trigger_stop_loss(
        env: Env,
        user: Address,
        trade_id: u64,
        asset_pair: u32,
    ) -> Result<bool, ContractError> {
        triggers::check_and_trigger_stop_loss(&env, user, trade_id, asset_pair)
    }

    pub fn set_take_profit_price(env: Env, user: Address, trade_id: u64, take_profit_price: i128) {
        user.require_auth();
        triggers::set_take_profit(&env, &user, trade_id, take_profit_price);
    }

    pub fn set_take_profit_price_with_pair(
        env: Env,
        user: Address,
        trade_id: u64,
        take_profit_price: i128,
        asset_pair: u32,
    ) {
        user.require_auth();
        triggers::set_take_profit(&env, &user, trade_id, take_profit_price);
        register_watch(&env, &user, trade_id, asset_pair);
    }

    pub fn check_and_trigger_take_profit(
        env: Env,
        user: Address,
        trade_id: u64,
        asset_pair: u32,
    ) -> Result<bool, ContractError> {
        triggers::check_and_trigger_take_profit(&env, user, trade_id, asset_pair)
    }

    // ── Copy trade ────────────────────────────────────────────────────────────

    pub fn get_insufficient_balance_detail(
        env: Env,
        user: Address,
    ) -> Option<InsufficientBalanceDetail> {
        let key = StorageKey::LastInsufficientBalance(user);
        env.storage().instance().get(&key)
    }

    /// Runs copy trade: balance check (incl. fee), position limit, then portfolio
    /// `record_copy_position`.
    pub fn execute_copy_trade(
        env: Env,
        user: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        user.require_auth();

        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }

        let lock_key = Symbol::new(&env, EXECUTION_LOCK);
        if env
            .storage()
            .temporary()
            .get::<_, bool>(&lock_key)
            .unwrap_or(false)
        {
            return Err(ContractError::ReentrancyDetected);
        }
        env.storage().temporary().set(&lock_key, &true);

        let portfolio: Address = env
            .storage()
            .instance()
            .get(&StorageKey::UserPortfolio)
            .ok_or(ContractError::NotInitialized)?;

        let fee = effective_estimated_fee(&env);
        let bal_key = StorageKey::LastInsufficientBalance(user.clone());
        match check_user_balance(&env, &user, &token, amount, fee) {
            Ok(()) => {
                env.storage().instance().remove(&bal_key);
            }
            Err(detail) => {
                env.storage().instance().set(&bal_key, &detail);
                return Err(ContractError::InsufficientBalance);
            }
        }

        let exempt = {
            let key = StorageKey::PositionLimitExempt(user.clone());
            env.storage().instance().get(&key).unwrap_or(false)
        };

        check_position_limit(&env, &portfolio, &user, exempt)?;

        let sym = Symbol::new(&env, RECORD_COPY_POSITION_FN);
        let mut args = Vec::<Val>::new(&env);
        args.push_back(user.into_val(&env));
        env.invoke_contract::<()>(&portfolio, &sym, args);

        env.storage()
            .temporary()
            .remove(&Symbol::new(&env, EXECUTION_LOCK));
        Ok(())
    }

    // ── SDEX router configuration ─────────────────────────────────────────────

    pub fn set_sdex_router(env: Env, router: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::SdexRouter, &router);
    }

    pub fn get_sdex_router(env: Env) -> Option<Address> {
        env.storage().instance().get(&StorageKey::SdexRouter)
    }

    pub fn swap(
        env: Env,
        from_token: Address,
        to_token: Address,
        amount: i128,
        min_received: i128,
    ) -> Result<i128, ContractError> {
        let router = env
            .storage()
            .instance()
            .get(&StorageKey::SdexRouter)
            .ok_or(ContractError::NotInitialized)?;
        execute_sdex_swap(&env, &router, &from_token, &to_token, amount, min_received)
    }

    pub fn swap_with_slippage(
        env: Env,
        from_token: Address,
        to_token: Address,
        amount: i128,
        max_slippage_bps: u32,
    ) -> Result<i128, ContractError> {
        let min_received = min_received_from_slippage(amount, max_slippage_bps)
            .ok_or(ContractError::InvalidAmount)?;
        Self::swap(env, from_token, to_token, amount, min_received)
    }

    // ── Keeper network interface ──────────────────────────────────────────────

    /// Configure the SAC token used to pay keeper rewards (admin only).
    pub fn set_keeper_reward_token(env: Env, token: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&StorageKey::KeeperRewardToken, &token);
    }

    /// Return all positions whose oracle price has already crossed their trigger
    /// threshold.  Callable by any keeper — no auth required.
    pub fn get_triggerable_positions(env: Env) -> Vec<TriggerablePosition> {
        get_triggerable_positions(&env).unwrap_or_else(|_| Vec::new(&env))
    }

    /// Accrue a keeper reward for `position_value` to `keeper`.
    /// Called internally after a successful trigger; emits `KeeperRewarded`.
    /// The reward is stored on-chain and can be claimed via `claim_keeper_reward`.
    pub fn accrue_keeper_reward(env: Env, keeper: Address, position_value: i128) {
        let reward = compute_keeper_reward(position_value);
        if reward == 0 {
            return;
        }
        let key = StorageKey::KeeperReward(keeper.clone());
        let current: i128 = env.storage().instance().get(&key).unwrap_or(0);
        let new_total = current.checked_add(reward).unwrap_or(current);
        env.storage().instance().set(&key, &new_total);
        env.events().publish(
            (Symbol::new(&env, "KeeperRewarded"), keeper.clone()),
            (reward, new_total),
        );
    }

    /// Return the accrued (unclaimed) reward balance for `keeper`.
    pub fn get_keeper_reward(env: Env, keeper: Address) -> i128 {
        env.storage()
            .instance()
            .get(&StorageKey::KeeperReward(keeper))
            .unwrap_or(0)
    }

    // ── Trade timeout ─────────────────────────────────────────────────────────

    /// Place a pending limit order with an automatic expiry of `TRADE_TIMEOUT_LEDGERS`.
    /// Callable by the user; stores the order in persistent storage.
    pub fn place_trade(
        env: Env,
        user: Address,
        trade_id: u64,
        amount: i128,
    ) -> Result<(), ContractError> {
        user.require_auth();
        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }
        let expires_at_ledger = env
            .ledger()
            .sequence()
            .checked_add(TRADE_TIMEOUT_LEDGERS)
            .ok_or(ContractError::InvalidAmount)?;
        let order = TradeOrder {
            trade_id,
            user,
            amount,
            expires_at_ledger,
            status: TradeStatus::Pending,
        };
        env.storage()
            .persistent()
            .set(&StorageKey::TradeOrder(trade_id), &order);
        Ok(())
    }

    /// Expire an unfilled trade whose timeout has elapsed. Callable by any keeper (no user auth).
    /// Removes the locked order and emits `TradeExpired { trade_id, user, amount_returned }`.
    ///
    /// Errors:
    /// - `TradeNotFound`      — no order with this `trade_id`
    /// - `TradeAlreadyFilled` — order status is `Filled`
    /// - `TradeNotExpired`    — timeout has not yet elapsed
    pub fn expire_trade(env: Env, trade_id: u64) -> Result<(), ContractError> {
        let order: TradeOrder = env
            .storage()
            .persistent()
            .get(&StorageKey::TradeOrder(trade_id))
            .ok_or(ContractError::TradeNotFound)?;

        if order.status == TradeStatus::Filled {
            return Err(ContractError::TradeAlreadyFilled);
        }

        if env.ledger().sequence() <= order.expires_at_ledger {
            return Err(ContractError::TradeNotExpired);
        }

        env.storage()
            .persistent()
            .remove(&StorageKey::TradeOrder(trade_id));

        env.events().publish(
            (Symbol::new(&env, "TradeExpired"), order.user.clone()),
            (trade_id, order.user.clone(), order.amount),
        );

        Ok(())
    }

    // ── Manual position exit ──────────────────────────────────────────────────

    /// Cancel a copy trade manually: executes a SDEX swap to close the position,
    /// records exit in UserPortfolio, and emits `TradeCancelled`.
    pub fn cancel_copy_trade(
        env: Env,
        caller: Address,
        user: Address,
        trade_id: u64,
        from_token: Address,
        to_token: Address,
        amount: i128,
        min_received: i128,
    ) -> Result<(), ContractError> {
        caller.require_auth();
        if caller != user {
            return Err(ContractError::Unauthorized);
        }

        let portfolio: Address = env
            .storage()
            .instance()
            .get(&StorageKey::UserPortfolio)
            .ok_or(ContractError::NotInitialized)?;

        let exists: bool = {
            let sym = Symbol::new(&env, "has_position");
            let mut args = Vec::<Val>::new(&env);
            args.push_back(user.clone().into_val(&env));
            args.push_back(trade_id.into_val(&env));
            env.invoke_contract::<bool>(&portfolio, &sym, args)
        };
        if !exists {
            return Err(ContractError::TradeNotFound);
        }

        let router: Address = env
            .storage()
            .instance()
            .get(&StorageKey::SdexRouter)
            .ok_or(ContractError::NotInitialized)?;

        let exit_price =
            execute_sdex_swap(&env, &router, &from_token, &to_token, amount, min_received)?;

        let realized_pnl = exit_price - amount;
        let close_sym = Symbol::new(&env, "close_position");
        let mut close_args = Vec::<Val>::new(&env);
        close_args.push_back(user.clone().into_val(&env));
        close_args.push_back(trade_id.into_val(&env));
        close_args.push_back(realized_pnl.into_val(&env));
        env.invoke_contract::<()>(&portfolio, &close_sym, close_args);

        env.events().publish(
            (Symbol::new(&env, "TradeCancelled"), user.clone()),
            (trade_id, exit_price, realized_pnl),
        );

        Ok(())
    }
}

#[cfg(test)]
mod test;
