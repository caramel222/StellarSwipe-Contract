#![no_std]

mod errors;
pub use errors::ContractError;

mod events;
pub use events::{FeeRateUpdated, FeesBurned, FeesClaimed, TreasuryWithdrawal, WithdrawalQueued};

mod rebates;

mod storage;
pub use storage::{
    get_admin, get_burn_rate, get_fee_rate, get_monthly_trade_volume, get_oracle_contract,
    get_pending_fees, get_queued_withdrawal, get_treasury_balance, is_initialized,
    remove_monthly_trade_volume, remove_queued_withdrawal, set_admin,
    set_burn_rate as set_burn_rate_storage, set_fee_rate as set_fee_rate_storage, set_initialized,
    set_monthly_trade_volume, set_oracle_contract as set_oracle_contract_storage, set_pending_fees,
    set_queued_withdrawal, set_treasury_balance, MonthlyTradeVolume, QueuedWithdrawal, StorageKey,
    MAX_BURN_RATE_BPS, MAX_FEE_RATE_BPS, MIN_FEE_RATE_BPS,
};

use soroban_sdk::{contract, contractimpl, token, Address, Env};

use stellar_swipe_common::Asset;
use stellar_swipe_common::SECONDS_PER_DAY;

#[cfg(test)]
mod test;

#[contract]
pub struct FeeCollector;

#[contractimpl]
impl FeeCollector {
    /// # Summary
    /// One-time contract initialization. Sets the admin address.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `admin`: Address that will hold admin privileges.
    ///
    /// # Returns
    /// `Ok(())` on success.
    ///
    /// # Errors
    /// - [`ContractError::AlreadyInitialized`] if the contract has already been initialized.
    pub fn initialize(env: Env, admin: Address) -> Result<(), ContractError> {
        admin.require_auth();
        if is_initialized(&env) {
            return Err(ContractError::AlreadyInitialized);
        }
        set_admin(&env, &admin);
        set_initialized(&env);
        Ok(())
    }

    /// # Summary
    /// Set the oracle contract address used for price-based fee calculations.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `oracle_contract`: Address of the oracle contract.
    ///
    /// # Returns
    /// `Ok(())` on success.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if the contract has not been initialized.
    pub fn set_oracle_contract(env: Env, oracle_contract: Address) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        set_oracle_contract_storage(&env, &oracle_contract);
        Ok(())
    }

    /// # Summary
    /// Returns the effective fee rate in basis points for a specific user,
    /// accounting for any volume-based rebates.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `user`: Address of the trader.
    ///
    /// # Returns
    /// Fee rate in basis points (e.g. `30` = 0.30%).
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if the contract has not been initialized.
    pub fn fee_rate_for_user(env: Env, user: Address) -> Result<u32, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(rebates::get_fee_rate_for_user(&env, &user))
    }

    /// # Summary
    /// Returns the 30-day rolling trade volume in USD for a user.
    /// Used to determine rebate tier eligibility.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `user`: Address of the trader.
    ///
    /// # Returns
    /// Volume in USD (scaled by asset decimals).
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if the contract has not been initialized.
    pub fn monthly_trade_volume(env: Env, user: Address) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(rebates::get_active_volume_usd(&env, &user))
    }

    /// # Summary
    /// Returns the current treasury balance for a given token.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `token`: SEP-41 token contract address.
    ///
    /// # Returns
    /// Balance in the token's native units.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] if the contract has not been initialized.
    pub fn treasury_balance(env: Env, token: Address) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(get_treasury_balance(&env, &token))
    }

    /// # Summary
    /// Queue a treasury withdrawal. The withdrawal becomes executable after a
    /// 24-hour timelock. Admin auth required.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `recipient`: Address that will receive the tokens.
    /// - `token`: SEP-41 token contract address.
    /// - `amount`: Amount to withdraw (must be > 0 and <= treasury balance).
    ///
    /// # Returns
    /// `Ok(())` on success. Emits a [`WithdrawalQueued`] event.
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — contract not initialized.
    /// - [`ContractError::InvalidAmount`] — amount <= 0.
    /// - [`ContractError::InsufficientTreasuryBalance`] — amount exceeds balance.
    pub fn queue_withdrawal(
        env: Env,
        recipient: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        if amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }
        if amount > get_treasury_balance(&env, &token) {
            return Err(ContractError::InsufficientTreasuryBalance);
        }
        let queued_at = env.ledger().timestamp();
        set_queued_withdrawal(
            &env,
            &QueuedWithdrawal {
                recipient: recipient.clone(),
                token: token.clone(),
                amount,
                queued_at,
            },
        );
        WithdrawalQueued {
            recipient: recipient.clone(),
            token: token.clone(),
            amount,
            available_at: queued_at + SECONDS_PER_DAY,
        }
        .publish(&env);
        Ok(())
    }

    /// # Summary
    /// Execute a previously queued treasury withdrawal after the 24-hour timelock.
    /// Admin auth required. Parameters must exactly match the queued withdrawal.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `recipient`: Must match the queued recipient.
    /// - `token`: Must match the queued token.
    /// - `amount`: Must match the queued amount.
    ///
    /// # Returns
    /// `Ok(())` on success. Transfers tokens and emits [`TreasuryWithdrawal`].
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — contract not initialized.
    /// - [`ContractError::WithdrawalNotQueued`] — no matching queued withdrawal.
    /// - [`ContractError::TimelockNotElapsed`] — 24-hour timelock has not passed.
    /// - [`ContractError::InsufficientTreasuryBalance`] — balance changed since queuing.
    pub fn withdraw_treasury_fees(
        env: Env,
        recipient: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();

        let queued = match get_queued_withdrawal(&env) {
            Some(q) if q.recipient == recipient && q.token == token && q.amount == amount => q,
            _ => return Err(ContractError::WithdrawalNotQueued),
        };

        if env.ledger().timestamp() < queued.queued_at + SECONDS_PER_DAY {
            return Err(ContractError::TimelockNotElapsed);
        }

        if amount > get_treasury_balance(&env, &token) {
            return Err(ContractError::InsufficientTreasuryBalance);
        }

        let new_balance = get_treasury_balance(&env, &token)
            .checked_sub(amount)
            .ok_or(ContractError::ArithmeticOverflow)?;

        token::Client::new(&env, &token).transfer(
            &env.current_contract_address(),
            &recipient,
            &amount,
        );

        set_treasury_balance(&env, &token, new_balance);
        remove_queued_withdrawal(&env);

        TreasuryWithdrawal {
            recipient: recipient.clone(),
            token: token.clone(),
            amount,
            remaining_balance: new_balance,
        }
        .publish(&env);

        Ok(())
    }

    /// Returns the current fee rate in basis points.
    pub fn fee_rate(env: Env) -> Result<u32, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(get_fee_rate(&env))
    }

    /// Admin-only: update the fee rate (in basis points).
    /// Validates: MIN_FEE_RATE_BPS <= new_rate_bps <= MAX_FEE_RATE_BPS.
    /// Change takes effect on the next trade — no retroactive application.
    pub fn set_fee_rate(env: Env, new_rate_bps: u32) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();

        if new_rate_bps > MAX_FEE_RATE_BPS {
            return Err(ContractError::FeeRateTooHigh);
        }
        if new_rate_bps < MIN_FEE_RATE_BPS {
            return Err(ContractError::FeeRateTooLow);
        }

        let old_rate = get_fee_rate(&env);
        set_fee_rate_storage(&env, new_rate_bps);

        FeeRateUpdated {
            old_rate,
            new_rate: new_rate_bps,
            updated_by: admin,
        }
        .publish(&env);

        Ok(())
    }

    /// Returns the current burn rate in basis points (default: 1000 = 10%).
    pub fn burn_rate(env: Env) -> Result<u32, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        Ok(get_burn_rate(&env))
    }

    /// Admin-only: set the percentage of collected fees to burn (in basis points).
    /// Max is 10_000 (100%). Change takes effect on the next fee collection.
    pub fn set_burn_rate(env: Env, new_rate_bps: u32) -> Result<(), ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        if new_rate_bps > MAX_BURN_RATE_BPS {
            return Err(ContractError::BurnRateTooHigh);
        }
        set_burn_rate_storage(&env, new_rate_bps);
        Ok(())
    }

    /// # Summary
    /// Collect a fee from a trader for a completed trade. Transfers the fee
    /// from the trader to this contract, burns the configured burn slice,
    /// and credits the remainder to the treasury.
    ///
    /// # Parameters
    /// - `env`: Soroban environment.
    /// - `trader`: Address of the trader (must authorize).
    /// - `token`: SEP-41 token used to pay the fee.
    /// - `trade_amount`: Gross trade amount (fee is calculated as a percentage).
    /// - `trade_asset`: Asset pair traded (used for volume tracking).
    ///
    /// # Returns
    /// The total fee amount collected (before burn).
    ///
    /// # Errors
    /// - [`ContractError::NotInitialized`] — contract not initialized.
    /// - [`ContractError::InvalidAmount`] — trade_amount <= 0.
    /// - [`ContractError::FeeRoundedToZero`] — fee rounds to zero at current rate.
    /// - [`ContractError::ArithmeticOverflow`] — overflow in fee calculation.
    pub fn collect_fee(
        env: Env,
        trader: Address,
        token: Address,
        trade_amount: i128,
        trade_asset: Asset,
    ) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        trader.require_auth();

        if trade_amount <= 0 {
            return Err(ContractError::InvalidAmount);
        }

        let fee_rate = rebates::get_fee_rate_for_user(&env, &trader);
        let fee_amount = trade_amount
            .checked_mul(fee_rate as i128)
            .and_then(|amount| amount.checked_div(10_000))
            .ok_or(ContractError::ArithmeticOverflow)?;

        if fee_amount <= 0 {
            return Err(ContractError::FeeRoundedToZero);
        }

        token::Client::new(&env, &token).transfer(
            &trader,
            &env.current_contract_address(),
            &fee_amount,
        );

        // Burn slice
        let burn_rate = get_burn_rate(&env);
        let burn_amount = fee_amount
            .checked_mul(burn_rate as i128)
            .and_then(|v| v.checked_div(10_000))
            .ok_or(ContractError::ArithmeticOverflow)?;
        let distributable = fee_amount
            .checked_sub(burn_amount)
            .ok_or(ContractError::ArithmeticOverflow)?;

        if burn_amount > 0 {
            token::Client::new(&env, &token).burn(&env.current_contract_address(), &burn_amount);
            FeesBurned {
                amount: burn_amount,
                token: token.clone(),
            }
            .publish(&env);
        }

        let updated_treasury_balance = get_treasury_balance(&env, &token)
            .checked_add(distributable)
            .ok_or(ContractError::ArithmeticOverflow)?;
        set_treasury_balance(&env, &token, updated_treasury_balance);

        rebates::record_trade_volume(&env, &trader, &trade_asset, trade_amount)?;

        Ok(fee_amount)
    }

    /// Claim all pending fee earnings for a provider and token.
    /// Returns the amount claimed (0 if no pending balance).
    pub fn claim_fees(env: Env, provider: Address, token: Address) -> Result<i128, ContractError> {
        if !is_initialized(&env) {
            return Err(ContractError::NotInitialized);
        }
        provider.require_auth();

        let amount = get_pending_fees(&env, &provider, &token);

        if amount > 0 {
            token::Client::new(&env, &token).transfer(
                &env.current_contract_address(),
                &provider,
                &amount,
            );
            set_pending_fees(&env, &provider, &token, 0);
        }

        FeesClaimed {
            provider: provider.clone(),
            token: token.clone(),
            amount,
        }
        .publish(&env);

        Ok(amount)
    }
}
