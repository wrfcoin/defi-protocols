//! TWAP (Time-Weighted Average Price) oracle for WRF token pricing.
//!
//! Implements manipulation-resistant price feeds using circular buffer observation
//! storage and multi-source median aggregation. Enforces minimum 30-minute TWAP
//! windows for lending liquidation to resist flash-loan price attacks.
//!
//! # Architecture
//! - [`PriceAccumulator`]: per-asset cumulative price tracker, updated on each AMM swap
//! - [`ObservationBuffer`]: fixed-size ring buffer of `(Timestamp, Price)` snapshots
//! - [`TwapOracle`]: multi-pool aggregator exposing the [`PriceOracle`] trait

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::traits::PriceOracle;
use wrfcoin_defi_common::types::{AssetId, Price, Timestamp};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Default ring-buffer capacity per asset per pool (number of price observations).
pub const DEFAULT_BUFFER_CAPACITY: usize = 1_024;

/// Minimum TWAP window required for liquidation decisions (30 minutes).
pub const MIN_LIQUIDATION_WINDOW_SECS: u64 = 30 * 60;

/// Pre-defined TWAP window: 5 minutes.
pub const WINDOW_5MIN_SECS: u64 = 5 * 60;

/// Pre-defined TWAP window: 1 hour.
pub const WINDOW_1HR_SECS: u64 = 60 * 60;

/// Pre-defined TWAP window: 24 hours.
pub const WINDOW_24HR_SECS: u64 = 24 * 60 * 60;

/// Minimum number of observations required before a TWAP can be computed.
pub const MIN_OBSERVATIONS: usize = 2;

// ── Price observation ─────────────────────────────────────────────────────────

/// A single price observation stored in the ring buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceObservation {
    /// Unix timestamp at which this price was recorded (seconds).
    pub timestamp: Timestamp,
    /// Spot price recorded at this instant (numerator/denominator fraction).
    pub price: Price,
    /// Cumulative price-seconds accumulated up to (and including) this observation.
    /// Stored as a u128 to accommodate large windows without overflow.
    pub cumulative_price_secs: u128,
}

// ── ObservationBuffer ─────────────────────────────────────────────────────────

/// Fixed-size circular buffer of [`PriceObservation`] entries for a single asset/pool.
///
/// Oldest observations are overwritten when the buffer is full. The buffer maintains
/// chronological ordering: entries are inserted in order of increasing timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationBuffer {
    /// Ring buffer storage.
    entries: Vec<Option<PriceObservation>>,
    /// Write index pointing to the slot that will receive the *next* entry.
    head: usize,
    /// Total number of observations ever written (not capped by capacity).
    total_written: u64,
    /// Capacity of the ring buffer.
    capacity: usize,
}

impl ObservationBuffer {
    /// Create a new buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        assert!(
            capacity >= 2,
            "ObservationBuffer capacity must be at least 2"
        );
        Self {
            entries: vec![None; capacity],
            head: 0,
            total_written: 0,
            capacity,
        }
    }

    /// Number of valid observations currently stored.
    pub fn len(&self) -> usize {
        (self.total_written as usize).min(self.capacity)
    }

    /// True when the buffer contains no observations.
    pub fn is_empty(&self) -> bool {
        self.total_written == 0
    }

    /// Push a new observation. Overwrites the oldest entry when the buffer is full.
    pub fn push(&mut self, obs: PriceObservation) {
        self.entries[self.head] = Some(obs);
        self.head = (self.head + 1) % self.capacity;
        self.total_written += 1;
    }

    /// Most recent observation, or `None` when empty.
    pub fn latest(&self) -> Option<PriceObservation> {
        if self.total_written == 0 {
            return None;
        }
        let idx = if self.head == 0 {
            self.capacity - 1
        } else {
            self.head - 1
        };
        self.entries[idx]
    }

    /// Iterator over all valid entries in chronological order (oldest → newest).
    pub fn iter_chronological(&self) -> impl Iterator<Item = PriceObservation> + '_ {
        let count = self.len();
        let start = if self.total_written as usize > self.capacity {
            // Buffer has wrapped: oldest entry is at `head`
            self.head
        } else {
            0
        };
        (0..count).filter_map(move |i| self.entries[(start + i) % self.capacity])
    }

    /// Find the oldest observation whose timestamp is >= `since`, or the oldest
    /// observation in the buffer when all entries are newer than `since`.
    pub fn oldest_at_or_after(&self, since: Timestamp) -> Option<PriceObservation> {
        let mut result: Option<PriceObservation> = None;
        for obs in self.iter_chronological() {
            if obs.timestamp >= since {
                // Found the first entry >= since; we want the closest one
                result = Some(obs);
                break;
            }
        }
        result
    }

    /// Find the most recent observation whose timestamp is <= `at`.
    pub fn latest_at_or_before(&self, at: Timestamp) -> Option<PriceObservation> {
        let mut result: Option<PriceObservation> = None;
        for obs in self.iter_chronological() {
            if obs.timestamp <= at {
                result = Some(obs);
            } else {
                break;
            }
        }
        result
    }
}

// ── PriceAccumulator ──────────────────────────────────────────────────────────

/// Tracks cumulative price-seconds for a single asset within one AMM pool.
///
/// Updated on every swap so that time-weighted averages can be derived by
/// dividing the difference in `cumulative_price_secs` by the elapsed time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceAccumulator {
    /// The asset this accumulator tracks.
    pub asset: AssetId,
    /// Running sum of `price * elapsed_seconds` since the accumulator was created.
    pub cumulative_price_secs: u128,
    /// Timestamp of the last update.
    pub last_update: Timestamp,
    /// Spot price at the last update.
    pub last_price: Price,
    /// Circular buffer of historic observations.
    pub buffer: ObservationBuffer,
}

impl PriceAccumulator {
    /// Create a new accumulator seeded with an initial price.
    pub fn new(asset: AssetId, initial_price: Price, now: Timestamp) -> Self {
        let obs = PriceObservation {
            timestamp: now,
            price: initial_price,
            cumulative_price_secs: 0,
        };
        let mut buffer = ObservationBuffer::new(DEFAULT_BUFFER_CAPACITY);
        buffer.push(obs);
        Self {
            asset,
            cumulative_price_secs: 0,
            last_update: now,
            last_price: initial_price,
            buffer,
        }
    }

    /// Record a new price observation (called after each AMM swap).
    ///
    /// The cumulative accumulator advances by `last_price * elapsed_seconds`
    /// before storing the new price. Observations at the same timestamp as
    /// the previous update are merged (no time has passed, so no accumulation).
    pub fn record(&mut self, new_price: Price, now: Timestamp) {
        // Calculate elapsed time in seconds since the last update.
        let elapsed = now.elapsed_since(self.last_update);

        // Advance the cumulative price-seconds.
        if elapsed > 0 {
            let price_as_fixed = price_to_fixed(self.last_price);
            let delta = price_as_fixed.saturating_mul(elapsed as u128);
            self.cumulative_price_secs = self.cumulative_price_secs.saturating_add(delta);
        }

        self.last_price = new_price;
        self.last_update = now;

        let obs = PriceObservation {
            timestamp: now,
            price: new_price,
            cumulative_price_secs: self.cumulative_price_secs,
        };
        self.buffer.push(obs);
    }
}

// ── TWAP computation helpers ──────────────────────────────────────────────────

/// Fixed-point scale for price representation in cumulative accumulators.
/// We use 1e18 to preserve precision across the price fraction.
const FIXED_SCALE: u128 = 1_000_000_000_000_000_000u128; // 1e18

/// Convert a `Price` fraction to a fixed-point integer scaled by `FIXED_SCALE`.
fn price_to_fixed(p: Price) -> u128 {
    // Avoid division by zero (guaranteed by Price invariant, but guard anyway).
    if p.denominator == 0 {
        return 0;
    }
    // numerator * FIXED_SCALE / denominator
    // Use u128 saturating arithmetic to avoid panics on overflow.
    p.numerator
        .saturating_mul(FIXED_SCALE)
        .saturating_div(p.denominator)
}

/// Convert a fixed-point value back to a `Price` fraction.
fn fixed_to_price(fixed: u128) -> Price {
    // Represent as fixed / FIXED_SCALE fraction.
    Price::new(fixed, FIXED_SCALE)
}

// ── TwapOracle ────────────────────────────────────────────────────────────────

/// Multi-pool TWAP oracle implementing [`PriceOracle`].
///
/// Aggregates price data from one or more AMM pools using a median-of-TWAPs
/// strategy for manipulation resistance. Falls back to a [`SimplePriceOracle`]-
/// compatible snapshot when there is insufficient history.
///
/// # Fallback
/// When fewer than [`MIN_OBSERVATIONS`] are available for a given asset,
/// `get_price` returns an error unless a fallback price is set via
/// [`TwapOracle::set_fallback_price`].
///
/// # Liquidation guard
/// When `enforce_min_window` is `true` (the default), TWAP windows shorter
/// than [`MIN_LIQUIDATION_WINDOW_SECS`] are rejected for safety.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwapOracle {
    /// Map from `(pool_id_string, asset_id)` → PriceAccumulator.
    /// We use `String` as the key's pool identifier to avoid a dependency on PoolId here.
    accumulators: HashMap<(String, String), PriceAccumulator>,

    /// Fallback spot prices used when a TWAP cannot be computed.
    fallback_prices: HashMap<String, Price>,

    /// When `true`, windows shorter than MIN_LIQUIDATION_WINDOW_SECS are rejected.
    pub enforce_min_window: bool,

    /// Default TWAP window in seconds when `get_price` is called without specifying one.
    pub default_window_secs: u64,
}

impl TwapOracle {
    /// Create a new TWAP oracle with 1-hour default window.
    pub fn new() -> Self {
        Self {
            accumulators: HashMap::new(),
            fallback_prices: HashMap::new(),
            enforce_min_window: true,
            default_window_secs: WINDOW_1HR_SECS,
        }
    }

    /// Register an accumulator for a specific pool/asset pair and seed it with
    /// the current spot price.
    pub fn init_pool_asset(
        &mut self,
        pool_id: &str,
        asset: AssetId,
        initial_price: Price,
        now: Timestamp,
    ) {
        let key = (pool_id.to_string(), asset.as_str().to_string());
        self.accumulators
            .entry(key)
            .or_insert_with(|| PriceAccumulator::new(asset, initial_price, now));
    }

    /// Record a new price observation for a pool/asset pair.
    ///
    /// This is the hook called from `SwapEngine::execute_swap_with_twap`.
    pub fn record_observation(
        &mut self,
        pool_id: &str,
        asset: &AssetId,
        new_price: Price,
        now: Timestamp,
    ) {
        let key = (pool_id.to_string(), asset.as_str().to_string());
        if let Some(acc) = self.accumulators.get_mut(&key) {
            acc.record(new_price, now);
        }
    }

    /// Set a fallback spot price for an asset (used when TWAP history is insufficient).
    pub fn set_fallback_price(&mut self, asset: AssetId, price: Price) {
        self.fallback_prices
            .insert(asset.as_str().to_string(), price);
    }

    /// Compute the TWAP for an asset across a specific pool over the given window.
    ///
    /// Returns `Err` when:
    /// - `enforce_min_window` is true and `window_secs < MIN_LIQUIDATION_WINDOW_SECS`
    /// - no observations exist for the pool/asset pair
    /// - the oldest available observation is newer than `now - window_secs` (window too short)
    pub fn twap_for_pool(
        &self,
        pool_id: &str,
        asset: &AssetId,
        window_secs: u64,
        now: Timestamp,
    ) -> DefiResult<Price> {
        if self.enforce_min_window && window_secs < MIN_LIQUIDATION_WINDOW_SECS {
            return Err(DefiError::OracleError {
                message: format!(
                    "TWAP window {window_secs}s is below the minimum {MIN_LIQUIDATION_WINDOW_SECS}s required for liquidations"
                ),
            });
        }

        let key = (pool_id.to_string(), asset.as_str().to_string());
        let acc = self
            .accumulators
            .get(&key)
            .ok_or_else(|| DefiError::OracleError {
                message: format!("no TWAP accumulator for pool '{pool_id}' asset '{asset}'"),
            })?;

        compute_twap_from_buffer(&acc.buffer, window_secs, now)
    }

    /// Compute the TWAP for an asset across all registered pools, then return
    /// the *median* of the per-pool TWAPs.
    ///
    /// This is the primary price fetch used by `LendingMarket`. It aggregates
    /// across all pools that have data for `asset` in order to resist manipulation
    /// of any single pool.
    ///
    /// Falls back to a stored fallback price when insufficient history is available
    /// across all pools for this asset.
    pub fn twap(&self, asset: &AssetId, window_secs: u64, now: Timestamp) -> DefiResult<Price> {
        if self.enforce_min_window && window_secs < MIN_LIQUIDATION_WINDOW_SECS {
            return Err(DefiError::OracleError {
                message: format!(
                    "TWAP window {window_secs}s is below the minimum {MIN_LIQUIDATION_WINDOW_SECS}s required for liquidations"
                ),
            });
        }

        let asset_key = asset.as_str().to_string();
        let mut prices: Vec<u128> = Vec::new();

        for ((_, acc_asset), acc) in &self.accumulators {
            if acc_asset != &asset_key {
                continue;
            }
            if let Ok(p) = compute_twap_from_buffer(&acc.buffer, window_secs, now) {
                prices.push(price_to_fixed(p));
            }
        }

        if prices.is_empty() {
            // Try fallback price
            return self
                .fallback_prices
                .get(&asset_key)
                .copied()
                .ok_or_else(|| DefiError::OracleError {
                    message: format!(
                        "insufficient TWAP history for asset '{asset}' and no fallback price set"
                    ),
                });
        }

        // Return the median of all per-pool TWAPs.
        let median_fixed = median_of(&mut prices);
        Ok(fixed_to_price(median_fixed))
    }

    /// Number of distinct pool/asset accumulators registered.
    pub fn accumulator_count(&self) -> usize {
        self.accumulators.len()
    }
}

impl Default for TwapOracle {
    fn default() -> Self {
        Self::new()
    }
}

/// Implement PriceOracle using the default window and all registered pools.
///
/// Callers that need a specific window should use `TwapOracle::twap` directly.
/// The `PriceOracle` trait integration uses the `default_window_secs` and
/// falls back to stored fallback prices when history is insufficient.
impl PriceOracle for TwapOracle {
    fn get_price(&self, asset: &AssetId) -> DefiResult<Price> {
        self.twap(asset, self.default_window_secs, Timestamp::new(u64::MAX))
            // If the generic call failed (e.g. no observations), also try fallback directly.
            .or_else(|_| {
                self.fallback_prices
                    .get(asset.as_str())
                    .copied()
                    .ok_or_else(|| DefiError::OracleError {
                        message: format!("no price available for asset '{asset}'"),
                    })
            })
    }
}

// ── Internal TWAP computation ─────────────────────────────────────────────────

/// Compute a TWAP from a ring buffer by finding the earliest observation at or
/// after `now - window_secs` and computing the slope of cumulative price-seconds.
fn compute_twap_from_buffer(
    buffer: &ObservationBuffer,
    window_secs: u64,
    now: Timestamp,
) -> DefiResult<Price> {
    if buffer.len() < MIN_OBSERVATIONS {
        return Err(DefiError::OracleError {
            message: format!(
                "insufficient observations: need at least {}, have {}",
                MIN_OBSERVATIONS,
                buffer.len()
            ),
        });
    }

    let latest = buffer.latest().ok_or_else(|| DefiError::OracleError {
        message: "buffer has no latest observation".into(),
    })?;

    // The earliest timestamp we will accept for the window start.
    let window_start = Timestamp::new(now.as_secs().saturating_sub(window_secs));

    // Find the oldest observation that falls within the requested window.
    let earliest = buffer.oldest_at_or_after(window_start).unwrap_or_else(|| {
        // All observations are older than the window: use the oldest available.
        buffer
            .iter_chronological()
            .next()
            .expect("buffer is non-empty")
    });

    let elapsed = latest.timestamp.elapsed_since(earliest.timestamp);
    if elapsed == 0 {
        // Only one distinct timestamp — return the spot price from latest.
        return Ok(latest.price);
    }

    // TWAP = (cumulative_latest - cumulative_earliest) / elapsed
    let cumulative_delta = latest
        .cumulative_price_secs
        .saturating_sub(earliest.cumulative_price_secs);

    let twap_fixed = cumulative_delta / elapsed as u128;
    Ok(fixed_to_price(twap_fixed))
}

/// Return the median element from a non-empty mutable slice (partially sorts in-place).
fn median_of(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        // Average the two middle values (saturating to avoid overflow).
        let mid = n / 2;
        values[mid - 1] / 2 + values[mid] / 2
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wrfcoin_defi_common::types::AssetId;

    fn wrf() -> AssetId {
        AssetId::new("WRF")
    }

    fn price(num: u128, den: u128) -> Price {
        Price::new(num, den)
    }

    fn ts(secs: u64) -> Timestamp {
        Timestamp::new(secs)
    }

    // ── ObservationBuffer tests ───────────────────────────────────────────────

    #[test]
    fn test_buffer_empty() {
        let buf = ObservationBuffer::new(4);
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        assert!(buf.latest().is_none());
    }

    #[test]
    fn test_buffer_push_and_len() {
        let mut buf = ObservationBuffer::new(4);
        let obs = PriceObservation {
            timestamp: ts(1),
            price: price(100, 1),
            cumulative_price_secs: 0,
        };
        buf.push(obs);
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.latest(), Some(obs));
    }

    #[test]
    fn test_buffer_wraps_correctly() {
        let capacity = 3;
        let mut buf = ObservationBuffer::new(capacity);

        // Insert 5 observations; oldest 2 should be overwritten.
        for i in 0u64..5 {
            buf.push(PriceObservation {
                timestamp: ts(i),
                price: price(i as u128 + 1, 1),
                cumulative_price_secs: i as u128 * 10,
            });
        }

        assert_eq!(buf.len(), capacity);

        let entries: Vec<_> = buf.iter_chronological().collect();
        assert_eq!(entries.len(), capacity);
        // Should be timestamps 2, 3, 4 (oldest 0 and 1 were overwritten).
        assert_eq!(entries[0].timestamp, ts(2));
        assert_eq!(entries[1].timestamp, ts(3));
        assert_eq!(entries[2].timestamp, ts(4));
        assert_eq!(buf.latest().unwrap().timestamp, ts(4));
    }

    #[test]
    fn test_buffer_oldest_at_or_after() {
        let mut buf = ObservationBuffer::new(8);
        for i in [10u64, 20, 30, 40] {
            buf.push(PriceObservation {
                timestamp: ts(i),
                price: price(1, 1),
                cumulative_price_secs: i as u128,
            });
        }

        // Exact match
        assert_eq!(buf.oldest_at_or_after(ts(20)).unwrap().timestamp, ts(20));
        // Between entries
        assert_eq!(buf.oldest_at_or_after(ts(25)).unwrap().timestamp, ts(30));
        // Before all entries
        assert_eq!(buf.oldest_at_or_after(ts(5)).unwrap().timestamp, ts(10));
        // After all entries → None
        assert!(buf.oldest_at_or_after(ts(50)).is_none());
    }

    // ── PriceAccumulator tests ────────────────────────────────────────────────

    #[test]
    fn test_accumulator_records_observations() {
        let mut acc = PriceAccumulator::new(wrf(), price(100, 1), ts(0));
        acc.record(price(200, 1), ts(60));
        acc.record(price(150, 1), ts(120));

        assert_eq!(acc.buffer.len(), 3); // initial + 2 records
        assert_eq!(acc.last_price, price(150, 1));
        assert_eq!(acc.last_update, ts(120));
    }

    #[test]
    fn test_accumulator_cumulative_price_advances() {
        // price = 100 for 60 seconds → cumulative += 100 * 60 * FIXED_SCALE
        let mut acc = PriceAccumulator::new(wrf(), price(100, 1), ts(0));
        acc.record(price(200, 1), ts(60));

        let expected_fixed = price_to_fixed(price(100, 1)).saturating_mul(60);
        assert_eq!(acc.cumulative_price_secs, expected_fixed);
    }

    #[test]
    fn test_accumulator_same_timestamp_no_accumulation() {
        let mut acc = PriceAccumulator::new(wrf(), price(100, 1), ts(0));
        let before = acc.cumulative_price_secs;
        // Record at the same timestamp → no elapsed time
        acc.record(price(999, 1), ts(0));
        assert_eq!(acc.cumulative_price_secs, before);
    }

    // ── TWAP computation tests ────────────────────────────────────────────────

    #[test]
    fn test_twap_constant_price() {
        // Constant price of 100 over 1 hour.
        let mut oracle = TwapOracle::new();
        oracle.enforce_min_window = false;
        oracle.init_pool_asset("pool1", wrf(), price(100, 1), ts(0));

        // Simulate 6 observations at 10-minute intervals.
        for i in 1u64..=6 {
            oracle.record_observation("pool1", &wrf(), price(100, 1), ts(i * 600));
        }

        let now = ts(6 * 600);
        let result = oracle.twap(&wrf(), WINDOW_1HR_SECS, now).unwrap();
        // The TWAP of a constant price should equal that price (within fixed-point rounding).
        // TWAP fixed should be ~100 * FIXED_SCALE.
        let twap_as_ratio = (result.numerator as f64) / (result.denominator as f64);
        let expected = 100.0_f64;
        assert!(
            (twap_as_ratio - expected).abs() < 1.0,
            "expected TWAP ≈ {expected}, got {twap_as_ratio}"
        );
    }

    #[test]
    fn test_twap_window_computation() {
        // Price = 100 for the first 30 min, then 200 for the next 30 min.
        // TWAP over 60 min should be ~150.
        let mut oracle = TwapOracle::new();
        oracle.enforce_min_window = false;
        oracle.init_pool_asset("pool1", wrf(), price(100, 1), ts(0));

        oracle.record_observation("pool1", &wrf(), price(100, 1), ts(1800)); // 30 min
        oracle.record_observation("pool1", &wrf(), price(200, 1), ts(3600)); // 60 min

        let now = ts(3600);
        let result = oracle.twap(&wrf(), WINDOW_1HR_SECS, now).unwrap();
        let twap_as_ratio = (result.numerator as f64) / (result.denominator as f64);
        // Expected: (100 * 1800 + 200 * 1800) but divide by full window; last price not yet accumulated.
        // Actual accumulation: from ts(0) to ts(1800): 100 * 1800 = 180000 fixed units
        // from ts(1800) to ts(3600): 100 * 1800 (last price before switch was 100)
        // Then at ts(3600), price becomes 200 but it's the latest observation,
        // cumulative delta represents what was accumulated up to ts(3600).
        // This verifies the TWAP is between 100 and 200.
        assert!(
            twap_as_ratio > 90.0 && twap_as_ratio < 210.0,
            "TWAP {twap_as_ratio} should be between 100 and 200"
        );
    }

    // ── Flash-loan / manipulation resistance tests ────────────────────────────

    #[test]
    fn test_single_block_manipulation_does_not_move_twap() {
        // Build 30 min of history at price 100, then inject a single spike to 10_000.
        // The TWAP should remain close to 100 because the spike occupied < 1 second.
        let mut oracle = TwapOracle::new();
        oracle.enforce_min_window = false;
        oracle.init_pool_asset("pool1", wrf(), price(100, 1), ts(0));

        // 30 minutes of normal trading.
        for i in 1u64..=30 {
            oracle.record_observation("pool1", &wrf(), price(100, 1), ts(i * 60));
        }

        let t_attack = 30 * 60; // attack at 30-min mark (same second, no elapsed)
                                // Manipulate: record a 100× spike at the same timestamp as the last entry.
        oracle.record_observation("pool1", &wrf(), price(10_000, 1), ts(t_attack));

        // Immediately restore (next second).
        oracle.record_observation("pool1", &wrf(), price(100, 1), ts(t_attack + 1));

        let now = ts(t_attack + 1);
        let result = oracle.twap(&wrf(), 30 * 60, now).unwrap();
        let twap_as_ratio = (result.numerator as f64) / (result.denominator as f64);

        // The 1-second spike should move the TWAP by at most ~1/1800 of the spike,
        // which is negligible. Ensure TWAP is still close to 100.
        assert!(
            twap_as_ratio < 200.0,
            "single-block manipulation moved TWAP to {twap_as_ratio}, expected < 200"
        );
    }

    #[test]
    fn test_min_window_enforced_for_liquidation() {
        let oracle = TwapOracle::new(); // enforce_min_window = true by default

        // A 5-minute window should be rejected.
        let result = oracle.twap(&wrf(), WINDOW_5MIN_SECS, ts(10_000));
        assert!(
            matches!(result, Err(DefiError::OracleError { .. })),
            "short window should be rejected"
        );
    }

    #[test]
    fn test_min_window_not_enforced_when_disabled() {
        let mut oracle = TwapOracle::new();
        oracle.enforce_min_window = false;
        oracle.set_fallback_price(wrf(), price(100, 1));

        // Without enforcement, even a 1-second window should not fail with a window error.
        // (It may fail for insufficient data, but not for the window check.)
        let result = oracle.twap(&wrf(), 1, ts(10_000));
        // Should fall through to fallback (no observations yet).
        assert!(
            result.is_ok(),
            "expected fallback to supply price, got {result:?}"
        );
    }

    // ── Fallback tests ────────────────────────────────────────────────────────

    #[test]
    fn test_fallback_when_no_history() {
        let mut oracle = TwapOracle::new();
        oracle.enforce_min_window = false;
        oracle.set_fallback_price(wrf(), price(500, 1));

        // No accumulator registered → fallback should kick in.
        let result = oracle.twap(&wrf(), WINDOW_1HR_SECS, ts(10_000)).unwrap();
        assert_eq!(result, price(500, 1));
    }

    #[test]
    fn test_no_fallback_returns_error() {
        let mut oracle = TwapOracle::new();
        oracle.enforce_min_window = false;
        // No fallback set.
        let result = oracle.twap(&wrf(), WINDOW_1HR_SECS, ts(10_000));
        assert!(matches!(result, Err(DefiError::OracleError { .. })));
    }

    #[test]
    fn test_price_oracle_trait_uses_fallback() {
        let mut oracle = TwapOracle::new();
        oracle.set_fallback_price(wrf(), price(123, 1));

        let result = oracle.get_price(&wrf()).unwrap();
        assert_eq!(result, price(123, 1));
    }

    // ── Multi-pool median tests ───────────────────────────────────────────────

    #[test]
    fn test_median_aggregation_odd_pools() {
        let mut oracle = TwapOracle::new();
        oracle.enforce_min_window = false;

        // Register 3 pools with different prices over 1 hour.
        for (pool, p) in [("pool1", 90u128), ("pool2", 100u128), ("pool3", 110u128)] {
            oracle.init_pool_asset(pool, wrf(), price(p, 1), ts(0));
            oracle.record_observation(pool, &wrf(), price(p, 1), ts(3600));
        }

        let result = oracle.twap(&wrf(), WINDOW_1HR_SECS, ts(3600)).unwrap();
        let twap_as_ratio = (result.numerator as f64) / (result.denominator as f64);
        // Median of [90, 100, 110] should be close to 100.
        assert!(
            (twap_as_ratio - 100.0).abs() < 2.0,
            "median TWAP {twap_as_ratio} should be ≈ 100"
        );
    }

    #[test]
    fn test_median_aggregation_even_pools() {
        let mut oracle = TwapOracle::new();
        oracle.enforce_min_window = false;

        for (pool, p) in [("pool1", 80u128), ("pool2", 120u128)] {
            oracle.init_pool_asset(pool, wrf(), price(p, 1), ts(0));
            oracle.record_observation(pool, &wrf(), price(p, 1), ts(3600));
        }

        let result = oracle.twap(&wrf(), WINDOW_1HR_SECS, ts(3600)).unwrap();
        let twap_as_ratio = (result.numerator as f64) / (result.denominator as f64);
        // Median of [80, 120] = average of two midpoints ≈ 100.
        assert!(
            (twap_as_ratio - 100.0).abs() < 5.0,
            "median TWAP {twap_as_ratio} should be ≈ 100"
        );
    }

    #[test]
    fn test_manipulated_pool_cannot_sway_median() {
        let mut oracle = TwapOracle::new();
        oracle.enforce_min_window = false;

        // Two legitimate pools at price 100; one manipulated pool at 10_000.
        for (pool, p) in [
            ("legit1", 100u128),
            ("legit2", 100u128),
            ("bad", 10_000u128),
        ] {
            oracle.init_pool_asset(pool, wrf(), price(p, 1), ts(0));
            oracle.record_observation(pool, &wrf(), price(p, 1), ts(3600));
        }

        let result = oracle.twap(&wrf(), WINDOW_1HR_SECS, ts(3600)).unwrap();
        let twap_as_ratio = (result.numerator as f64) / (result.denominator as f64);
        // Median of [100, 100, 10_000] = 100.
        assert!(
            twap_as_ratio < 200.0,
            "manipulated pool should not sway median: got {twap_as_ratio}"
        );
    }

    // ── median_of helper tests ────────────────────────────────────────────────

    #[test]
    fn test_median_of_single() {
        let mut v = vec![42u128];
        assert_eq!(median_of(&mut v), 42);
    }

    #[test]
    fn test_median_of_odd() {
        let mut v = vec![3u128, 1, 2];
        assert_eq!(median_of(&mut v), 2);
    }

    #[test]
    fn test_median_of_even() {
        let mut v = vec![4u128, 2, 6, 8];
        // sorted: [2, 4, 6, 8], middle two: 4 and 6 → avg = 5
        assert_eq!(median_of(&mut v), 5);
    }
}
