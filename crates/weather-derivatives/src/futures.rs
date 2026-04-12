//! Weather futures: open positions, settlement queue, and futures book.

use crate::error::{DerivativesError, DerivativesResult};
use crate::index::WeatherIndex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::types::{Address, Amount};

/// Unique identifier for a weather futures contract definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FutureId(pub u64);

impl std::fmt::Display for FutureId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "future#{}", self.0)
    }
}

/// Status of a weather future.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FutureStatus {
    /// Accepting new positions.
    Open,
    /// Observation window closed; awaiting oracle settlement.
    PendingSettlement,
    /// Settled against oracle reading.
    Settled {
        /// Final index value scaled by 1_000_000.
        final_index_scaled: i64,
    },
}

/// A weather futures contract specification.
///
/// `tick_size` is the minimum price movement in WRF base units.
/// The settlement value equals `final_index_scaled / 1_000_000 * tick_value_wrf`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherFuture {
    pub id: FutureId,
    /// The weather index this future tracks.
    pub index: WeatherIndex,
    /// Block height at which the oracle reading is taken for settlement.
    /// Must equal or exceed the observation window's `end_block`.
    pub settlement_block: u64,
    /// Minimum price movement in WRF base units (u128).
    pub tick_size: Amount,
    pub status: FutureStatus,
}

impl WeatherFuture {
    pub fn new(
        id: FutureId,
        index: WeatherIndex,
        settlement_block: u64,
        tick_size: Amount,
    ) -> DerivativesResult<Self> {
        if tick_size.is_zero() {
            return Err(DerivativesError::InvalidParameter {
                reason: "tick_size must be non-zero".into(),
            });
        }
        if settlement_block < index.window().end_block {
            return Err(DerivativesError::InvalidParameter {
                reason: format!(
                    "settlement_block {} must be >= observation window end {}",
                    settlement_block,
                    index.window().end_block
                ),
            });
        }
        Ok(Self {
            id,
            index,
            settlement_block,
            tick_size,
            status: FutureStatus::Open,
        })
    }

    pub fn is_open(&self) -> bool {
        matches!(self.status, FutureStatus::Open)
    }

    pub fn is_settled(&self) -> bool {
        matches!(self.status, FutureStatus::Settled { .. })
    }
}

/// A trader's position on a weather future.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuturePosition {
    pub future_id: FutureId,
    pub trader: Address,
    /// Positive = long, negative stored as separate `is_long` flag here.
    pub is_long: bool,
    /// Number of contracts (integer lots).
    pub contracts: u64,
    /// Entry price (index value scaled by 1_000_000) at which position was opened.
    pub entry_price_scaled: i64,
    /// Settled P&L in WRF base units, populated after settlement.
    pub settled_pnl: Option<i128>,
}

impl FuturePosition {
    pub fn new(
        future_id: FutureId,
        trader: Address,
        is_long: bool,
        contracts: u64,
        entry_price_scaled: i64,
    ) -> Self {
        Self {
            future_id,
            trader,
            is_long,
            contracts,
            entry_price_scaled,
            settled_pnl: None,
        }
    }

    /// Compute P&L in WRF base units given a final index value.
    ///
    /// P&L = (final - entry) × contracts × tick_size  (long)
    ///     = (entry - final) × contracts × tick_size  (short)
    ///
    /// Returns an i128 to handle negative P&L.
    pub fn compute_pnl(&self, final_index_scaled: i64, tick_size: Amount) -> i128 {
        let diff = final_index_scaled - self.entry_price_scaled;
        let signed_diff = if self.is_long { diff } else { -diff };
        // Convert: signed_diff is in units of 1_000_000 of index value.
        // We divide by 1_000_000 to get actual index units, then multiply by tick_size.
        let tick_val = tick_size.0 as i128;
        let contracts = self.contracts as i128;
        // signed_diff / 1_000_000 * contracts * tick_val
        (signed_diff as i128) * contracts * tick_val / 1_000_000
    }
}

/// Manages all active weather futures contracts and their positions.
#[derive(Debug, Default)]
pub struct FuturesBook {
    futures: HashMap<FutureId, WeatherFuture>,
    /// Positions keyed by (future_id, trader).
    positions: HashMap<(FutureId, Address), FuturePosition>,
    next_id: u64,
}

impl FuturesBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create and register a new weather future.
    pub fn create_future(
        &mut self,
        index: WeatherIndex,
        settlement_block: u64,
        tick_size: Amount,
    ) -> DerivativesResult<FutureId> {
        let id = FutureId(self.next_id);
        self.next_id += 1;
        let future = WeatherFuture::new(id, index, settlement_block, tick_size)?;
        self.futures.insert(id, future);
        Ok(id)
    }

    /// Open a position on an existing future.
    pub fn open_position(
        &mut self,
        future_id: FutureId,
        trader: Address,
        is_long: bool,
        contracts: u64,
        entry_price_scaled: i64,
    ) -> DerivativesResult<()> {
        let future = self
            .futures
            .get(&future_id)
            .ok_or(DerivativesError::FutureNotFound { id: future_id.0 })?;

        if !future.is_open() {
            return Err(DerivativesError::AlreadySettled { id: future_id.0 });
        }

        let position =
            FuturePosition::new(future_id, trader, is_long, contracts, entry_price_scaled);
        self.positions.insert((future_id, trader), position);
        Ok(())
    }

    pub fn get_future(&self, id: FutureId) -> Option<&WeatherFuture> {
        self.futures.get(&id)
    }

    pub fn get_position(&self, future_id: FutureId, trader: Address) -> Option<&FuturePosition> {
        self.positions.get(&(future_id, trader))
    }

    /// Returns all futures whose settlement_block <= current_block and that are not yet settled.
    pub fn pending_settlement_futures(&self, current_block: u64) -> Vec<FutureId> {
        self.futures
            .iter()
            .filter(|(_, f)| {
                matches!(
                    f.status,
                    FutureStatus::Open | FutureStatus::PendingSettlement
                ) && f.settlement_block <= current_block
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// Mark a future as pending settlement (observation window closed).
    pub fn mark_pending(
        &mut self,
        future_id: FutureId,
        current_block: u64,
    ) -> DerivativesResult<()> {
        let future = self
            .futures
            .get_mut(&future_id)
            .ok_or(DerivativesError::FutureNotFound { id: future_id.0 })?;

        if future.settlement_block > current_block {
            return Err(DerivativesError::NotYetExpired {
                settlement_block: future.settlement_block,
                current_block,
            });
        }
        future.status = FutureStatus::PendingSettlement;
        Ok(())
    }

    /// Settle a future at a final index value, computing P&L for all positions.
    pub fn settle(
        &mut self,
        future_id: FutureId,
        final_index_scaled: i64,
    ) -> DerivativesResult<Vec<(Address, i128)>> {
        let tick_size = {
            let future = self
                .futures
                .get_mut(&future_id)
                .ok_or(DerivativesError::FutureNotFound { id: future_id.0 })?;

            if future.is_settled() {
                return Err(DerivativesError::AlreadySettled { id: future_id.0 });
            }

            let tick = future.tick_size;
            future.status = FutureStatus::Settled { final_index_scaled };
            tick
        };

        let mut payouts = Vec::new();
        for ((fid, trader), pos) in self.positions.iter_mut() {
            if *fid != future_id {
                continue;
            }
            let pnl = pos.compute_pnl(final_index_scaled, tick_size);
            pos.settled_pnl = Some(pnl);
            payouts.push((*trader, pnl));
        }
        Ok(payouts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{GeographicRegion, ObservationWindow, TemperatureIndexKind, WeatherIndex};

    fn addr(n: u8) -> Address {
        let mut b = [0u8; 32];
        b[31] = n;
        Address::from_bytes(b)
    }

    fn make_temp_index(start: u64, end: u64) -> WeatherIndex {
        WeatherIndex::Temperature {
            kind: TemperatureIndexKind::CDD,
            base_temp_scaled: 65_000_000,
            region: GeographicRegion::new("US-NYC"),
            window: ObservationWindow::new(start, end),
        }
    }

    #[test]
    fn test_create_future() {
        let mut book = FuturesBook::new();
        let id = book
            .create_future(make_temp_index(100, 200), 200, Amount::new(1_000))
            .unwrap();
        assert_eq!(id.0, 0);
        assert!(book.get_future(id).unwrap().is_open());
    }

    #[test]
    fn test_open_and_settle_long_position() {
        let mut book = FuturesBook::new();
        let id = book
            .create_future(make_temp_index(100, 200), 200, Amount::new(1_000_000))
            .unwrap();

        // Long position: entry at 50 CDD
        book.open_position(id, addr(1), true, 10, 50_000_000)
            .unwrap();

        // Settle at 60 CDD → profit = (60-50) × 10 × 1_000_000 / 1_000_000 = 10_000_000
        let payouts = book.settle(id, 60_000_000).unwrap();
        assert_eq!(payouts.len(), 1);
        assert_eq!(payouts[0].1, 10 * 10_000_000i128);
    }

    #[test]
    fn test_open_and_settle_short_position() {
        let mut book = FuturesBook::new();
        let id = book
            .create_future(make_temp_index(100, 200), 200, Amount::new(1_000_000))
            .unwrap();

        // Short at 80 CDD, settles at 60 CDD → profit = (80-60) × 5 × tick
        book.open_position(id, addr(2), false, 5, 80_000_000)
            .unwrap();
        let payouts = book.settle(id, 60_000_000).unwrap();
        assert_eq!(payouts[0].1, 20i128 * 5 * 1_000_000);
    }

    #[test]
    fn test_settle_twice_fails() {
        let mut book = FuturesBook::new();
        let id = book
            .create_future(make_temp_index(100, 200), 200, Amount::new(1_000))
            .unwrap();
        book.settle(id, 50_000_000).unwrap();
        assert!(book.settle(id, 50_000_000).is_err());
    }

    #[test]
    fn test_pending_settlement_futures() {
        let mut book = FuturesBook::new();
        let id = book
            .create_future(make_temp_index(100, 200), 200, Amount::new(1_000))
            .unwrap();
        assert!(book.pending_settlement_futures(199).is_empty());
        let pending = book.pending_settlement_futures(200);
        assert!(pending.contains(&id));
    }
}
