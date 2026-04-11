use std::collections::HashMap;

use crate::error::{PredictionError, PredictionResult};
use crate::lmsr;
use crate::market::ForecastMarket;
use crate::payout::{PayoutEngine, PayoutResult};
use crate::position::PositionLedger;
use crate::resolution::{ForecastVerificationReceipt, ResolutionEngine, ResolutionResult};
use wrfcoin_defi_common::types::Amount;
use wrfcoin_amm::fees::FeeManager;

/// Top-level prediction market engine: holds all markets, the position ledger,
/// and the fee manager for protocol fee accounting.
pub struct PredictionMarketEngine {
    pub markets: HashMap<u64, ForecastMarket>,
    pub ledger: PositionLedger,
    pub fee_manager: FeeManager,
    next_market_id: u64,
}

impl PredictionMarketEngine {
    pub fn new(protocol_fee_share_bps: u64) -> Self {
        Self {
            markets: HashMap::new(),
            ledger: PositionLedger::new(),
            fee_manager: FeeManager::new(protocol_fee_share_bps),
            next_market_id: 1,
        }
    }

    /// Register a market and return its assigned id.
    pub fn add_market(&mut self, mut market: ForecastMarket) -> u64 {
        let id = self.next_market_id;
        market.id = id;
        self.markets.insert(id, market);
        self.next_market_id += 1;
        id
    }

    /// Buy `delta` shares of `outcome_index` in market `market_id`.
    ///
    /// Returns the cost in WRF base units charged to the user.
    pub fn buy_outcome(
        &mut self,
        market_id: u64,
        outcome_index: usize,
        delta: f64,
        user: wrfcoin_defi_common::types::Address,
    ) -> PredictionResult<Amount> {
        let market = self
            .markets
            .get_mut(&market_id)
            .ok_or(PredictionError::MarketNotFound { market_id })?;

        if !market.is_open() {
            return Err(PredictionError::AlreadyResolved { market_id });
        }
        if outcome_index >= market.num_outcomes() {
            return Err(PredictionError::InvalidOutcome {
                outcome_index,
                num_outcomes: market.num_outcomes(),
            });
        }

        // Compute LMSR cost.
        let cost_f64 = lmsr::buy_cost(&market.q, market.lmsr_b, outcome_index, delta)?;
        let cost_units = lmsr::lmsr_cost_to_base_units(cost_f64);
        let cost_amount = Amount::new(cost_units);

        // Update market state.
        market.q[outcome_index] += delta;
        market.total_stake = market
            .total_stake
            .checked_add(cost_amount)
            .ok_or_else(|| PredictionError::Overflow {
                operation: "total_stake accumulation".into(),
            })?;

        // Update user position.
        let pos = self.ledger.get_or_create_mut(user, market_id, outcome_index);
        pos.add_shares(delta, cost_amount);

        Ok(cost_amount)
    }

    /// Sell `delta` shares of `outcome_index` in market `market_id`.
    ///
    /// Returns the proceeds in WRF base units returned to the user.
    pub fn sell_outcome(
        &mut self,
        market_id: u64,
        outcome_index: usize,
        delta: f64,
        user: wrfcoin_defi_common::types::Address,
    ) -> PredictionResult<Amount> {
        // Check user has enough shares.
        {
            let pos = self
                .ledger
                .get(&user, market_id, outcome_index)
                .ok_or(PredictionError::PositionNotFound {
                    market_id,
                    outcome_index,
                })?;
            if pos.shares < delta {
                return Err(PredictionError::InsufficientFunds {
                    required: delta as u128,
                    available: pos.shares as u128,
                });
            }
        }

        let market = self
            .markets
            .get_mut(&market_id)
            .ok_or(PredictionError::MarketNotFound { market_id })?;

        if !market.is_open() {
            return Err(PredictionError::AlreadyResolved { market_id });
        }

        let proceeds_f64 = lmsr::sell_proceeds(&market.q, market.lmsr_b, outcome_index, delta)?;
        let proceeds_units = proceeds_f64.floor() as u128; // round down: user gets floor
        let proceeds_amount = Amount::new(proceeds_units);

        // Update market state.
        market.q[outcome_index] -= delta;
        market.total_stake = market
            .total_stake
            .checked_sub(proceeds_amount)
            .unwrap_or(Amount::ZERO);

        // Update user position.
        let pos = self.ledger.get_or_create_mut(user, market_id, outcome_index);
        pos.remove_shares(delta);

        Ok(proceeds_amount)
    }

    /// Resolve a market using an oracle receipt.
    pub fn resolve_market(
        &mut self,
        market_id: u64,
        receipt: &ForecastVerificationReceipt,
        current_height: u64,
    ) -> PredictionResult<ResolutionResult> {
        let market = self
            .markets
            .get_mut(&market_id)
            .ok_or(PredictionError::MarketNotFound { market_id })?;
        ResolutionEngine::resolve(market, receipt, current_height)
    }

    /// Compute payouts for a resolved market.
    pub fn compute_payouts(&self, market_id: u64) -> PredictionResult<PayoutResult> {
        let market = self
            .markets
            .get(&market_id)
            .ok_or(PredictionError::MarketNotFound { market_id })?;
        let result = PayoutEngine::compute_payouts(market, &self.ledger)?;

        Ok(result)
    }

    /// LMSR implied price (probability) of an outcome.
    pub fn price(&self, market_id: u64, outcome_index: usize) -> PredictionResult<f64> {
        let market = self
            .markets
            .get(&market_id)
            .ok_or(PredictionError::MarketNotFound { market_id })?;
        lmsr::price(&market.q, market.lmsr_b, outcome_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::{ForecastMarket, WeatherMetric};
    use crate::resolution::ForecastVerificationReceipt;
    use wrfcoin_defi_common::types::Address;

    fn addr(byte: u8) -> Address {
        let mut b = [0u8; 32];
        b[31] = byte;
        Address::from_bytes(b)
    }

    fn default_engine() -> PredictionMarketEngine {
        PredictionMarketEngine::new(1667)
    }

    fn add_binary_market(engine: &mut PredictionMarketEngine, resolution_height: u64, threshold: f64) -> u64 {
        let market = ForecastMarket::new_binary(
            0, // id assigned by engine
            [0u8; 32],
            "North-Atlantic",
            WeatherMetric::Temperature,
            resolution_height,
            threshold,
            100.0,
            30,
            Address::zero(),
        );
        engine.add_market(market)
    }

    fn add_scalar_market(engine: &mut PredictionMarketEngine, resolution_height: u64) -> u64 {
        let market = ForecastMarket::new_scalar(
            0,
            [0u8; 32],
            "Pacific",
            WeatherMetric::Precipitation,
            resolution_height,
            100.0,
            30,
            Address::zero(),
        );
        engine.add_market(market)
    }

    fn receipt(accuracy_score: f64, at_height: u64) -> ForecastVerificationReceipt {
        ForecastVerificationReceipt {
            forecast_manifest_hash: [0u8; 32],
            verified_at_height: at_height,
            accuracy_score,
        }
    }

    // Test 1: LMSR cost monotonically increases as more shares are bought.
    #[test]
    fn test_lmsr_cost_monotone() {
        let mut engine = default_engine();
        let mid = add_binary_market(&mut engine, 100, 2.0);

        let cost1 = engine.buy_outcome(mid, 0, 10.0, addr(1)).unwrap();
        let cost2 = engine.buy_outcome(mid, 0, 10.0, addr(2)).unwrap();
        let cost3 = engine.buy_outcome(mid, 0, 10.0, addr(3)).unwrap();

        assert!(cost2 >= cost1, "successive buys should cost at least as much");
        assert!(cost3 >= cost2, "LMSR cost is convex");
    }

    // Test 2: Binary market — buying Yes shares increases Yes price, decreases No price.
    #[test]
    fn test_binary_price_dynamics() {
        let mut engine = default_engine();
        let mid = add_binary_market(&mut engine, 100, 2.0);

        let p_yes_before = engine.price(mid, 0).unwrap();
        let p_no_before = engine.price(mid, 1).unwrap();

        engine.buy_outcome(mid, 0, 20.0, addr(1)).unwrap();

        let p_yes_after = engine.price(mid, 0).unwrap();
        let p_no_after = engine.price(mid, 1).unwrap();

        assert!(p_yes_after > p_yes_before, "Yes price should rise after buying Yes");
        assert!(p_no_after < p_no_before, "No price should fall after buying Yes");
    }

    // Test 3: Resolution with RMSE=1.5 < threshold=2.0 → Yes; RMSE=2.5 → No.
    #[test]
    fn test_resolution_yes_and_no() {
        let mut engine = default_engine();

        let mid_yes = add_binary_market(&mut engine, 50, 2.0);
        let r_yes = engine.resolve_market(mid_yes, &receipt(1.5, 50), 50).unwrap();
        assert_eq!(r_yes.winning_outcome, 0, "RMSE 1.5 < 2.0 should be Yes");

        let mid_no = add_binary_market(&mut engine, 50, 2.0);
        let r_no = engine.resolve_market(mid_no, &receipt(2.5, 50), 50).unwrap();
        assert_eq!(r_no.winning_outcome, 1, "RMSE 2.5 >= 2.0 should be No");
    }

    // Test 4: Pro-rata payout — 60/40 split after fee deduction.
    #[test]
    fn test_binary_pro_rata_payout() {
        use crate::market::MarketStatus;

        let mut engine = default_engine();
        let mid = add_binary_market(&mut engine, 100, 2.0);

        // Manually inject shares to set up exact 60/40 split.
        {
            let pos_a = engine.ledger.get_or_create_mut(addr(1), mid, 0);
            pos_a.add_shares(60.0, Amount::ZERO);
            let pos_b = engine.ledger.get_or_create_mut(addr(2), mid, 0);
            pos_b.add_shares(40.0, Amount::ZERO);
        }

        let market = engine.markets.get_mut(&mid).unwrap();
        market.total_stake = Amount::new(10_000);
        market.status = MarketStatus::Resolved {
            winning_outcome: 0,
            accuracy_score: 1.5,
        };

        let result = engine.compute_payouts(mid).unwrap();
        // fee = 10_000 * 30 / 10_000 = 30; distributable = 9_970
        assert_eq!(result.protocol_fee, Amount::new(30));

        let payout_a = result.payouts.iter().find(|p| p.user == addr(1)).unwrap();
        let payout_b = result.payouts.iter().find(|p| p.user == addr(2)).unwrap();

        // A: floor(9970 * 60/100) = floor(5982) = 5982
        // B: floor(9970 * 40/100) = floor(3988) = 3988
        assert_eq!(payout_a.amount, Amount::new(5982));
        assert_eq!(payout_b.amount, Amount::new(3988));
    }

    // Test 5: Scalar market — higher accuracy score → higher payout.
    #[test]
    fn test_scalar_payout_proportional_to_accuracy() {
        use crate::market::MarketStatus;

        let mut engine = default_engine();
        let mid = add_scalar_market(&mut engine, 50);

        {
            let pos = engine.ledger.get_or_create_mut(addr(1), mid, 0);
            pos.add_shares(100.0, Amount::ZERO);
        }

        // Low accuracy (score=0.3)
        {
            let market = engine.markets.get_mut(&mid).unwrap();
            market.total_stake = Amount::new(10_000);
            market.status = MarketStatus::Resolved {
                winning_outcome: 0,
                accuracy_score: 0.3,
            };
        }
        let low_result = engine.compute_payouts(mid).unwrap();
        let low_payout = low_result.payouts[0].amount;

        // Reset to higher accuracy (score=0.9)
        {
            let market = engine.markets.get_mut(&mid).unwrap();
            market.status = MarketStatus::Resolved {
                winning_outcome: 0,
                accuracy_score: 0.9,
            };
        }
        let high_result = engine.compute_payouts(mid).unwrap();
        let high_payout = high_result.payouts[0].amount;

        assert!(
            high_payout > low_payout,
            "higher accuracy score should yield higher payout: low={} high={}",
            low_payout,
            high_payout
        );
    }

    // Test 6: Market cannot be resolved before resolution_height.
    #[test]
    fn test_resolution_too_early() {
        let mut engine = default_engine();
        let mid = add_binary_market(&mut engine, 200, 2.0);
        let err = engine.resolve_market(mid, &receipt(1.0, 100), 100).unwrap_err();
        assert!(matches!(err, PredictionError::ResolutionTooEarly { .. }));
    }

    // Test 7: Double-resolution guard.
    #[test]
    fn test_double_resolution_guard() {
        let mut engine = default_engine();
        let mid = add_binary_market(&mut engine, 100, 2.0);
        engine.resolve_market(mid, &receipt(1.5, 100), 100).unwrap();
        let err = engine.resolve_market(mid, &receipt(1.5, 100), 100).unwrap_err();
        assert!(matches!(err, PredictionError::AlreadyResolved { .. }));
    }
}
