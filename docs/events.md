# Signal Registry Events

## SignalSubmitted
Emitted when a new signal is submitted via create_signal().

**Topics:** (`signal_registry`, `signal_submitted`)
**Data:** `{ signal_id: u64, provider: Address, asset_pair: String, action: SignalAction, expiry: u64, risk_rating: RiskLevel }`

## SignalExpired
Emitted when signal status changes to Expired during expiry check.

**Topics:** (`signal_registry`, `signal_expired`)
**Data:** `{ signal_id: u64, provider: Address, expired_at: u64 }`

## Other Events
- `trade_executed`
- `signal_status_changed`
- `provider_stats_updated`
- `follow_gained`
- `emergency_paused`
- etc. (see events.rs)
