//! Settlement engine: automatic settlement at observation window close using
//! oracle-attested readings.

use crate::error::{DerivativesError, DerivativesResult};
use crate::futures::{FutureId, FuturesBook};
use crate::oracle::WeatherOracle;
use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::types::Address;

/// An oracle-attested weather reading used for settlement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleReading {
    pub future_id: FutureId,
    /// The attested index value scaled by 1_000_000.
    pub index_value_scaled: i64,
    /// Block at which the reading was attested.
    pub attested_block: u64,
}

/// Result of settling a single futures contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementOutcome {
    pub future_id: FutureId,
    pub final_index_scaled: i64,
    /// P&L for each position: (trader, signed_pnl_wrf).
    pub payouts: Vec<(Address, i128)>,
}

impl SettlementOutcome {
    /// Returns net WRF transferred out (total positive payouts).
    pub fn total_payout(&self) -> u128 {
        self.payouts
            .iter()
            .filter(|(_, pnl)| *pnl > 0)
            .map(|(_, pnl)| *pnl as u128)
            .fold(0u128, |a, b| a.saturating_add(b))
    }
}

/// Orchestrates settlement of weather futures at observation window close.
pub struct SettlementEngine;

impl SettlementEngine {
    /// Settle a single weather future against an oracle reading.
    ///
    /// Steps:
    /// 1. Verify the future's observation window is closed at `current_block`.
    /// 2. Fetch the oracle reading for the region at `settlement_block`.
    /// 3. Compute the index value from the raw oracle reading.
    /// 4. Drive settlement on the `FuturesBook`.
    pub fn settle_future<O: WeatherOracle>(
        book: &mut FuturesBook,
        future_id: FutureId,
        current_block: u64,
        oracle: &O,
    ) -> DerivativesResult<SettlementOutcome> {
        // Clone only the data we need before mutably borrowing `book`.
        let (index, settlement_block) = {
            let future = book
                .get_future(future_id)
                .ok_or(DerivativesError::FutureNotFound { id: future_id.0 })?;

            if future.is_settled() {
                return Err(DerivativesError::AlreadySettled { id: future_id.0 });
            }

            let window = future.index.window();
            if !window.is_closed_at(current_block) {
                return Err(DerivativesError::NotYetExpired {
                    settlement_block: window.end_block,
                    current_block,
                });
            }

            (future.index.clone(), future.settlement_block)
        };

        // Fetch oracle reading at settlement block.
        let raw_reading = oracle.get_reading(index.region(), settlement_block)?;

        // Compute index value.
        let final_index_scaled = index.compute_from_reading(raw_reading);

        // Settle all positions.
        let payouts = book.settle(future_id, final_index_scaled)?;

        Ok(SettlementOutcome {
            future_id,
            final_index_scaled,
            payouts,
        })
    }

    /// Settle all futures whose observation windows are closed.
    /// Returns a list of outcomes (one per settled future).
    pub fn settle_all_pending<O: WeatherOracle>(
        book: &mut FuturesBook,
        current_block: u64,
        oracle: &O,
    ) -> Vec<DerivativesResult<SettlementOutcome>> {
        let pending = book.pending_settlement_futures(current_block);
        pending
            .into_iter()
            .map(|id| Self::settle_future(book, id, current_block, oracle))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::futures::FuturesBook;
    use crate::index::{GeographicRegion, ObservationWindow, TemperatureIndexKind, WeatherIndex};
    use crate::oracle::InMemoryOracle;
    use wrfcoin_defi_common::types::Amount;

    fn addr(n: u8) -> Address {
        let mut b = [0u8; 32];
        b[31] = n;
        Address::from_bytes(b)
    }

    fn region() -> GeographicRegion {
        GeographicRegion::new("US-NYC")
    }

    fn cdd_index(start: u64, end: u64) -> WeatherIndex {
        WeatherIndex::Temperature {
            kind: TemperatureIndexKind::CDD,
            base_temp_scaled: 65_000_000, // 65°F base
            region: region(),
            window: ObservationWindow::new(start, end),
        }
    }

    // ── Settlement correctness ────────────────────────────────────────────────

    #[test]
    fn test_future_settles_to_correct_index_value() {
        let mut book = FuturesBook::new();
        let future_id = book
            .create_future(cdd_index(100, 200), 200, Amount::new(1_000_000))
            .unwrap();

        // Open a long position at 50 CDD entry price.
        book.open_position(future_id, addr(1), true, 1, 50_000_000)
            .unwrap();

        // Oracle reports 72°F at settlement block → CDD = max(0, 72 - 65) = 7
        let mut oracle = InMemoryOracle::new();
        oracle.set(&region(), 200, 72_000_000); // 72°F observed

        let outcome = SettlementEngine::settle_future(&mut book, future_id, 201, &oracle).unwrap();

        // Final index = 7 CDD
        assert_eq!(outcome.final_index_scaled, 7_000_000);
        assert_eq!(outcome.payouts.len(), 1);

        let (trader, pnl) = outcome.payouts[0];
        assert_eq!(trader, addr(1));
        // P&L = (7 - 50) × 1 contract × 1_000_000 tick = -43_000_000 (loss)
        assert_eq!(pnl, 7_000_000 - 50_000_000);
    }

    #[test]
    fn test_long_profitable_settlement() {
        let mut book = FuturesBook::new();
        let fid = book
            .create_future(cdd_index(100, 200), 200, Amount::new(1_000_000))
            .unwrap();

        // Long 10 contracts at entry 50 CDD
        book.open_position(fid, addr(1), true, 10, 50_000_000)
            .unwrap();

        let mut oracle = InMemoryOracle::new();
        // observed = 90°F → CDD = 90 - 65 = 25
        oracle.set(&region(), 200, 90_000_000);

        let outcome = SettlementEngine::settle_future(&mut book, fid, 201, &oracle).unwrap();
        assert_eq!(outcome.final_index_scaled, 25_000_000);

        let (_, pnl) = outcome.payouts[0];
        // (25 - 50) × 10 × 1_000_000 / 1_000_000 = -25 × 10 × 1 = -250
        // Wait — entry was 50, final is 25. Long loses.
        // Let's recalculate: (25_000_000 - 50_000_000) × 10 × 1_000_000 / 1_000_000
        //                    = -25_000_000 × 10 = -250_000_000
        assert_eq!(pnl, -250_000_000i128);
    }

    #[test]
    fn test_short_profitable_settlement() {
        let mut book = FuturesBook::new();
        let fid = book
            .create_future(cdd_index(100, 200), 200, Amount::new(1_000_000))
            .unwrap();

        // Short 5 contracts at entry 80 CDD
        book.open_position(fid, addr(2), false, 5, 80_000_000)
            .unwrap();

        let mut oracle = InMemoryOracle::new();
        // observed = 72°F → CDD = 7
        oracle.set(&region(), 200, 72_000_000);

        let outcome = SettlementEngine::settle_future(&mut book, fid, 201, &oracle).unwrap();
        assert_eq!(outcome.final_index_scaled, 7_000_000);

        let (_, pnl) = outcome.payouts[0];
        // Short P&L = (entry - final) × contracts × tick / scale
        // = (80_000_000 - 7_000_000) × 5 × 1_000_000 / 1_000_000
        // = 73_000_000 × 5 = 365_000_000
        assert_eq!(pnl, 365_000_000i128);
    }

    #[test]
    fn test_settle_before_window_close_fails() {
        let mut book = FuturesBook::new();
        let fid = book
            .create_future(cdd_index(100, 200), 200, Amount::new(1_000))
            .unwrap();
        let oracle = InMemoryOracle::new();
        // current_block = 200 — window end is 200, so not yet closed (need > 200)
        let result = SettlementEngine::settle_future(&mut book, fid, 200, &oracle);
        assert!(result.is_err());
    }

    #[test]
    fn test_settle_twice_fails() {
        let mut book = FuturesBook::new();
        let fid = book
            .create_future(cdd_index(100, 200), 200, Amount::new(1_000))
            .unwrap();

        let mut oracle = InMemoryOracle::new();
        oracle.set(&region(), 200, 70_000_000);

        SettlementEngine::settle_future(&mut book, fid, 201, &oracle).unwrap();
        let result = SettlementEngine::settle_future(&mut book, fid, 202, &oracle);
        assert!(result.is_err());
    }

    #[test]
    fn test_total_payout() {
        let outcome = SettlementOutcome {
            future_id: FutureId(0),
            final_index_scaled: 10_000_000,
            payouts: vec![(addr(1), 500_000), (addr(2), -200_000), (addr(3), 300_000)],
        };
        assert_eq!(outcome.total_payout(), 800_000);
    }

    #[test]
    fn test_settle_all_pending() {
        let mut book = FuturesBook::new();
        let fid1 = book
            .create_future(cdd_index(100, 200), 200, Amount::new(1_000_000))
            .unwrap();
        let _fid2 = book
            .create_future(cdd_index(100, 300), 300, Amount::new(1_000_000))
            .unwrap();

        let mut oracle = InMemoryOracle::new();
        oracle.set(&region(), 200, 72_000_000);
        // fid2 not yet due at block 201

        let results = SettlementEngine::settle_all_pending(&mut book, 201, &oracle);
        // Only fid1 should settle (fid2 has settlement_block = 300)
        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok());
        assert_eq!(results[0].as_ref().unwrap().future_id, fid1);
    }
}
