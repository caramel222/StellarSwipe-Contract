# Fee Rounding Analysis

**Scope:** `contracts/fee_collector/src/lib.rs` — `collect_fee`  
**Date:** 2026-04-23  
**Status:** Audited and standardized

---

## Rounding Strategy

All fee arithmetic uses Rust integer division, which truncates toward zero (rounds down).
The strategy is applied deliberately per calculation:

| Calculation | Direction | Rationale |
|---|---|---|
| `fee_amount = trade_amount * fee_rate / 10_000` | Round **down** | User-favorable: trader never pays more than the exact rate |
| `burn_amount = fee_amount * burn_rate / 10_000` | Round **down** | Provider-favorable: `distributable = fee_amount - burn_amount` is effectively rounded up |
| `distributable = fee_amount - burn_amount` | Exact subtraction | No rounding; conserves every stroop |

---

## Dust Analysis

### Definition
Dust is any amount credited to the contract's token balance that can never be withdrawn or burned — permanently locked.

### Result: No Dust

The split `burn_amount + distributable = fee_amount` is exact by construction:

```
distributable = fee_amount - burn_amount
             = fee_amount - floor(fee_amount * burn_rate / 10_000)
```

Every stroop of `fee_amount` is accounted for:
- `burn_amount` stroops are burned via `token::burn`
- `distributable` stroops are added to `treasury_balance`

The treasury balance is fully withdrawable by the admin via `withdraw_treasury_fees`.  
There is no residual balance that cannot be claimed.

### Sub-stroop Remainder (fee calculation)

When `trade_amount * fee_rate` is not divisible by `10_000`, the remainder is discarded.
This remainder is never transferred to the contract — the trader simply pays the truncated
`fee_amount`. The contract's token balance increases by exactly `fee_amount`, so no dust
enters the contract from this truncation.

---

## Worked Examples

### Example 1 — Standard trade
```
trade_amount = 1_000_000 stroops
fee_rate     = 30 bps (0.30%)
burn_rate    = 1_000 bps (10%)

fee_amount   = 1_000_000 * 30 / 10_000 = 3_000   (exact, no truncation)
burn_amount  = 3_000 * 1_000 / 10_000  = 300      (exact)
distributable= 3_000 - 300             = 2_700
```
Conservation: 300 + 2_700 = 3_000 ✓

### Example 2 — Non-round fee (user-favorable truncation)
```
trade_amount = 9_999 stroops
fee_rate     = 30 bps

fee_amount   = 9_999 * 30 / 10_000 = 29.997 → 29  (truncated, user saves 0.997 stroops)
```
The 0.997-stroop remainder is never transferred; the contract receives exactly 29 stroops.

### Example 3 — Non-round burn (provider-favorable)
```
fee_amount   = 2_333 stroops
burn_rate    = 3_333 bps (33.33%)

burn_amount  = 2_333 * 3_333 / 10_000 = 777.5889 → 777  (truncated)
distributable= 2_333 - 777            = 1_556
```
Conservation: 777 + 1_556 = 2_333 ✓  
The provider receives 1 extra stroop compared to exact 33.33% split — provider-favorable.

### Example 4 — Fee rounds to zero (rejected)
```
trade_amount = 9_999 stroops
fee_rate     = 1 bps

fee_amount   = 9_999 * 1 / 10_000 = 0  → ContractError::FeeRoundedToZero
```
Trades too small to produce a non-zero fee are rejected, preventing zero-fee abuse.

---

## Invariants

1. `burn_amount + distributable == fee_amount` for every `collect_fee` call.
2. `fee_amount >= 1` — enforced by `FeeRoundedToZero` guard.
3. `treasury_balance` only increases by `distributable` and only decreases by admin-initiated withdrawals — no other code path touches it.
4. The contract's on-chain token balance equals `treasury_balance + sum(pending_fees)` at all times.

---

## Test Coverage

Unit tests in `contracts/fee_collector/src/test.rs` verify:

| Test | What it checks |
|---|---|
| `test_fee_rounds_down_user_favorable` | Fee truncates; user never overpays |
| `test_burn_rounds_down_no_dust` | Burn truncates; burn + treasury == fee |
| `test_no_unwithdrawable_dust_accumulates` | Non-round burn rate; conservation holds |
| `test_fee_rounded_to_zero_error` | Sub-minimum trades are rejected |
