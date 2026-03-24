# Momentum Strategy - Quick Reference & API Guide

## Strategy Configuration Template

```rust
let strategy = MomentumStrategy {
    strategy_id: 1,
    user: user_address,
    asset_pairs: vec![
        AssetPair { base: 1, quote: 2 },  // USDC/XLM
        AssetPair { base: 3, quote: 4 },  // BTC/USD
        // ... up to 10 assets
    ],
    
    // Period for calculating momentum (days)
    momentum_period_days: 7,
    
    // Minimum ROC to trigger signal (basis points: 0-10000)
    // 1000 = 10%, 500 = 5%, 10000 = 100%
    min_momentum_threshold: 1000,
    
    // Require all indicators aligned before trading
    trend_confirmation_required: true,
    
    // Position size as % of portfolio (0-10000 = 0-100%)
    position_size_pct: 1000,  // 10% of portfolio per position
    
    // Trailing stop distance (0-10000 = 0-100%)
    trailing_stop_pct: 1000,  // Stop 10% below highest price
    
    // Enable momentum-based rebalancing
    ranking_enabled: true,
};
```

## Function Quick Reference

### 1. Calculate Momentum Indicators

```rust
let indicators = calculate_momentum_indicators(
    env,
    &price_history,
    strategy.momentum_period_days,
)?;

// Returns:
// - rate_of_change: -10000 to +10000 (basis points)
// - rsi: 0 to 10000 (0-100%)
// - macd: positive or negative
// - macd_signal: positive or negative
// - trend_strength: 0 to 10000 (0-100%)
```

**Interpretation**:
| Indicator | Bullish | Bearish | Neutral |
|-----------|---------|---------|---------|
| ROC | > 0 | < 0 | = 0 |
| RSI | > 5000 | < 5000 | = 5000 |
| MACD | > Signal | < Signal | = Signal |
| Trend | > 6000 | > 6000 (any) | < 6000 |

### 2. Generate Trading Signal

```rust
if let Some(signal) = check_momentum_signals(
    env,
    &strategy,
    asset_pair,
    &prices,
)? {
    // signal.direction: TradeDirection::Buy or::Sell
    // signal.confidence: 0-10000 (0-100%)
    // signal.momentum_strength: absolute ROC value
}
```

**Signal Requirements**:
- Momentum must exceed `min_momentum_threshold`
- If `trend_confirmation_required`:
  - RSI must be > 5000 (bullish) OR < 5000 (bearish)
  - MACD must be above/below signal line
  - Trend strength must be > 6000 (>60%)

### 3. Execute Trade

```rust
let trade_id = execute_momentum_trade(
    env,
    strategy_id,
    signal,
    current_price,     // Price at execution time
    portfolio_value,   // Total portfolio value
)?;

// Position created with:
// - amount = portfolio_value * position_size_pct / 10000
// - entry_price = current_price
// - trailing_stop = current_price * (1 - trailing_stop_pct/10000)
```

**Returns**: Trade ID (u64) for position tracking

### 4. Update Trailing Stops

```rust
let closed_positions = update_trailing_stops(env, strategy_id)?;

// For each position:
// - If price > highest_price: update highest, adjust stop upward
// - If price < stop: close position, add to closed_positions
// - Return list of closed AssetPairs
```

**Behavior**:
```
Scenario 1: Price increases
- Entry: $100, Stop: $90
- Price moves to $110
- New Stop: $99 (10% below new high)
- Profit protected while capturing upside

Scenario 2: Price increases then reverses
- Price: $110 → $98
- Current Stop: $99
- Result: Position closed, profit locked
```

### 5. Rank Assets

```rust
let ranked = rank_assets_by_momentum(
    env,
    &strategy,
    &prices_map,
)?;

// Returns Vec<(AssetPair, i128)>
// Sorted by momentum score (descending)
// Example:
// [(PAIR1, 5000), (PAIR2, 3000), (PAIR3, 1500), ...]
```

**Ranking Formula**:
```
score = rate_of_change + (trend_strength / 10)
```

### 6. Rebalance Portfolio

```rust
rebalance_by_momentum_rank(
    env,
    strategy_id,
    &ranked_assets,
    5,  // Hold top 5 assets
)?;

// Closes positions not in top 5
// Prepares to open positions in top 5
```

## Parameter Tuning Guide

### Momentum Period (`momentum_period_days`)
- **5-7 days**: Fast, responsive to recent trends (scalping)
- **14-21 days**: Balanced (swing trading)
- **30+ days**: Slow, trend following (position trading)

### Minimum Threshold (`min_momentum_threshold`)
- **500** (5%): Sensitive, many signals (high false positives)
- **1000** (10%): Balanced
- **2000+** (20%+): Conservative, fewer signals (higher quality)

### Trend Confirmation (`trend_confirmation_required`)
- **true**: Use when markets are choppy (fewer false signals)
- **false**: Use in trending markets (more trades)

### Position Size (`position_size_pct`)
- **500** (5%): Conservative, many positions possible
- **1000** (10%): Balanced risk/return
- **2000+** (20%+): Aggressive

### Trailing Stop (`trailing_stop_pct`)
- **500** (5%): Tight stop, protect gains quickly
- **1000** (10%): Balanced
- **2000+** (20%+): Allow more pullback

## Signal Quality Assessment

**High Confidence Signal** (> 8000):
- Strong ROC (> 3000 basis points)
- Extreme RSI (> 7000 or < 3000)
- Bullish MACD crossover
- Strong trend (> 6000)
- **Action**: Full position size

**Medium Confidence Signal** (5000-8000):
- Moderate ROC (1000-3000 basis points)
- MACD aligned
- Decent trend (4000-6000)
- **Action**: 75% position size

**Low Confidence Signal** (2000-5000):
- Weak ROC (< 1000 basis points)
- Marginal trend strength
- Single indicator confirmation
- **Action**: 50% position size or skip

## Common Trading Scenarios

### Scenario 1: Strong Uptrend
```
Indicators:
- ROC: +4500 (45% momentum)
- RSI: 7800 (extreme bullish)
- MACD: Above signal (bullish)
- Trend: 8500 (very strong)

Signal: BUY with 9200+ confidence
Action: Open full position with tight trailing stop
```

### Scenario 2: Weak Recovery
```
Indicators:
- ROC: +800 (8% momentum)
- RSI: 5200 (just above neutral)
- MACD: Marginal above signal
- Trend: 5200 (weak)

If min_momentum_threshold = 1000:
- REJECTED (ROC < threshold)

If min_momentum_threshold = 500:
- Signal generated with low confidence
- Consider reducing position size
```

### Scenario 3: Choppy Market
```
Indicators:
- ROC: +1500 (15% momentum)
- RSI: 5500 (neutral, not extreme)
- MACD: Mixed signals
- Trend: 4500 (weak, choppy)

If trend_confirmation_required = true:
- REJECTED (trend < 6000 threshold)

If trend_confirmation_required = false:
- Signal generated but medium confidence
- Trailing stop becomes critical
```

## Integration with Auto Trade Contract

### From auto_trade library:

```rust
// Use in execute_momentum_trade:
let portfolio_value = risk::calculate_portfolio_value(env, &user);

// Check position limits:
risk::check_position_limit(
    env,
    &user,
    asset_id,
    position_amount,
    current_price,
    &config,
)?;

// Record trade history:
history::record_trade(
    env,
    &user,
    signal_id,
    asset_id,
    position_amount,
    current_price,
    fee,
    status,
);
```

## Performance Considerations

### Gas Optimization
- Price history limited to recent data (7 days = ~500 snapshots)
- Map-based position storage for O(1) lookup
- Minimal iteration for small # of assets (< 10)

### Calculation Complexity
| Operation | Complexity | Notes |
|-----------|-----------|-------|
| ROC | O(1) | Two price points |
| RSI | O(n) | n = lookback period (14) |
| MACD | O(n) | n = 26 period |
| Trend | O(20) | Fixed 20-price window |
| Rank | O(n²) | n = # assets (typically 4-10) |

**Time Estimate**: ~1-2ms per 10-asset ranking on Stellar

## Testing & Validation

### Run All Tests
```bash
cd stellar-swipe/contracts/auto_trade
cargo test strategies::momentum::tests
```

### Key Tests to Verify
1. **Rate of Change**: Positive/negative momentum detection
2. **RSI**: Bullish/bearish threshold crossings
3. **MACD**: Crossover signals
4. **Trend Strength**: Strong vs weak trend detection
5. **Signal Generation**: Buy/sell signal accuracy
6. **Trade Execution**: Position creation and storage
7. **Trailing Stops**: Dynamic stop adjustment
8. **Ranking**: Asset momentum ordering
9. **Rebalancing**: Top-N asset selection
10. **Error Handling**: Insufficient data, position conflicts

## Troubleshooting

### No Signals Being Generated
1. Check `min_momentum_threshold` - may be too high
2. Verify price data has sufficient history (> period_days)
3. If `trend_confirmation_required` = true, check trend strength < 6000
4. Inspect ROC calculation - may be insufficient momentum

### Trailing Stops Not Triggering
1. Verify `update_trailing_stops()` is being called regularly
2. Check `trailing_stop_pct` is not 0
3. Confirm price data is being updated
4. Verify positions exist in strategy

### High False Signal Rate
1. Enable `trend_confirmation_required: true`
2. Increase `min_momentum_threshold`
3. Reduce `momentum_period_days` for faster trend detection
4. Check market is not choppy (use trend strength filter)

## References

- **ROC Indicator**: https://en.wikipedia.org/wiki/Rate_of_change_(finance)
- **RSI Indicator**: https://en.wikipedia.org/wiki/Relative_strength_index
- **MACD Indicator**: https://en.wikipedia.org/wiki/MACD
- **Trailing Stops**: https://www.investopedia.com/terms/t/trailingstop.asp
- **Momentum Trading**: https://www.investopedia.com/terms/m/momentum.asp
