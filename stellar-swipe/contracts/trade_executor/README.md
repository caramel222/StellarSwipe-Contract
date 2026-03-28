# Trade executor

## Copy trade position limit (`risk_gates.rs`)

Before opening a new copy position, [`TradeExecutorContract::execute_copy_trade`] calls [`risk_gates::check_position_limit`], which:

1. Returns `Ok(())` if the user is on the admin **position-limit whitelist** (instance key `PositionLimitExempt(user) == true`).
2. Otherwise invokes **`get_open_position_count(user) -> u32`** on the configured **user portfolio** contract via `Env::invoke_contract`.
3. Returns `ContractError::PositionLimitReached` when `open_count >= MAX_POSITIONS_PER_USER` (default **20**).

The check runs **before** `record_copy_position` is invoked on the portfolio, so no executor-side state changes happen when the limit applies.

### Portfolio contract ABI

- `get_open_position_count(user: Address) -> u32` — required for the limit check.
- `record_copy_position(user: Address)` — called after a successful check (void return). Your portfolio contract should persist the new open position here (or equivalent).

### Admin

- `set_user_portfolio` — portfolio contract address.
- `set_position_limit_exempt(user, exempt)` — per-user bypass of the cap.
