use serde::{Deserialize, Serialize};

use crate::error::{PredictionError, PredictionResult};
use crate::market::{ForecastMarket, MarketKind, MarketStatus};
use crate::position::PositionLedger;
use wrfcoin_defi_common::types::{Address, Amount};

/// A single user's payout result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPayout {
    pub user: Address,
    pub amount: Amount,
}

/// Aggregate payout result for a market.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayoutResult {
    pub market_id: u64,
    /// Individual payouts to winners.
    pub payouts: Vec<UserPayout>,
    /// Total fee deducted and sent to protocol.
    pub protocol_fee: Amount,
    /// Winning outcome index.
    pub winning_outcome: usize,
    /// Accuracy score from oracle.
    pub accuracy_score: f64,
}

/// Computes and distributes payouts after a market resolves.
pub struct PayoutEngine;

impl PayoutEngine {
    /// Compute pro-rata payouts for a resolved market.
    ///
    /// # Binary market
    /// The entire `total_stake` pool (minus fee) is distributed pro-rata
    /// among winners (holders of the winning outcome), in proportion to
    /// their share count.
    ///
    /// # Scalar market
    /// Each participant's payout is proportional to `accuracy_score` ∈ [0,1]
    /// times their stake fraction of the total pool.  The accuracy_score is
    /// expected to be a normalised value in [0, 1] (e.g. 1 / (1 + RMSE)).
    ///
    /// # Fee
    /// `fee_bps` basis points of `total_stake` are deducted first and
    /// allocated to the protocol treasury.
    pub fn compute_payouts(
        market: &ForecastMarket,
        ledger: &PositionLedger,
    ) -> PredictionResult<PayoutResult> {
        let (winning_outcome, accuracy_score) = match &market.status {
            MarketStatus::Resolved {
                winning_outcome,
                accuracy_score,
            } => (*winning_outcome, *accuracy_score),
            MarketStatus::Open => {
                return Err(PredictionError::NotResolved {
                    market_id: market.id,
                });
            }
        };

        let total_stake = market.total_stake;

        // Deduct protocol fee.
        let fee_amount = total_stake
            .mul_fraction(market.fee_bps as u128, 10_000)
            .ok_or_else(|| PredictionError::Overflow {
                operation: "fee deduction".into(),
            })?;
        let distributable = total_stake
            .checked_sub(fee_amount)
            .ok_or_else(|| PredictionError::Overflow {
                operation: "distributable after fee".into(),
            })?;

        let payouts = match market.kind {
            MarketKind::Binary => {
                Self::binary_payouts(market, ledger, winning_outcome, distributable)?
            }
            MarketKind::Scalar => {
                Self::scalar_payouts(market, ledger, accuracy_score, distributable)?
            }
        };

        if payouts.is_empty() {
            return Err(PredictionError::NoWinners);
        }

        Ok(PayoutResult {
            market_id: market.id,
            payouts,
            protocol_fee: fee_amount,
            winning_outcome,
            accuracy_score,
        })
    }

    /// Pro-rata distribution of `distributable` among winners of `winning_outcome`.
    fn binary_payouts(
        market: &ForecastMarket,
        ledger: &PositionLedger,
        winning_outcome: usize,
        distributable: Amount,
    ) -> PredictionResult<Vec<UserPayout>> {
        let winners = ledger.positions_for_outcome(market.id, winning_outcome);
        if winners.is_empty() {
            return Err(PredictionError::NoWinners);
        }

        // Total winning shares.
        let total_shares: f64 = winners.iter().map(|p| p.shares).sum();
        if total_shares <= 0.0 {
            return Err(PredictionError::NoWinners);
        }

        let distributable_f64 = distributable.0 as f64;
        let mut payouts = Vec::with_capacity(winners.len());

        for pos in &winners {
            if pos.shares <= 0.0 {
                continue;
            }
            let fraction = pos.shares / total_shares;
            let raw = distributable_f64 * fraction;
            let payout_units = raw.floor() as u128;
            if payout_units > 0 {
                payouts.push(UserPayout {
                    user: pos.user,
                    amount: Amount::new(payout_units),
                });
            }
        }

        Ok(payouts)
    }

    /// Scalar market: each holder's payout = distributable * (accuracy_score) * (their_shares / total_shares).
    ///
    /// `accuracy_score` is normalised to [0, 1] (higher = more accurate = higher payout).
    /// This models a payoff where a perfect forecast (score=1.0) returns the full distributable
    /// pool pro-rata, and a zero score (score=0.0) returns nothing.
    fn scalar_payouts(
        market: &ForecastMarket,
        ledger: &PositionLedger,
        accuracy_score: f64,
        distributable: Amount,
    ) -> PredictionResult<Vec<UserPayout>> {
        // Scalar markets have one outcome (index 0).
        let all_positions = ledger.positions_for_outcome(market.id, 0);
        if all_positions.is_empty() {
            return Err(PredictionError::NoWinners);
        }

        let total_shares: f64 = all_positions.iter().map(|p| p.shares).sum();
        if total_shares <= 0.0 {
            return Err(PredictionError::NoWinners);
        }

        // Clamp accuracy_score to [0, 1].
        let score = accuracy_score.clamp(0.0, 1.0);
        let distributable_f64 = distributable.0 as f64;

        let mut payouts = Vec::with_capacity(all_positions.len());
        for pos in &all_positions {
            if pos.shares <= 0.0 {
                continue;
            }
            let fraction = pos.shares / total_shares;
            let raw = distributable_f64 * score * fraction;
            let payout_units = raw.floor() as u128;
            if payout_units > 0 {
                payouts.push(UserPayout {
                    user: pos.user,
                    amount: Amount::new(payout_units),
                });
            }
        }

        Ok(payouts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::{ForecastMarket, MarketStatus, WeatherMetric};
    use crate::position::PositionLedger;
    use wrfcoin_defi_common::types::Address;

    fn addr(byte: u8) -> Address {
        let mut b = [0u8; 32];
        b[31] = byte;
        Address::from_bytes(b)
    }

    fn resolved_binary(winning: usize) -> ForecastMarket {
        let mut m = ForecastMarket::new_binary(
            1,
            [0u8; 32],
            "Region",
            WeatherMetric::Temperature,
            100,
            2.0,
            100.0,
            30, // fee 0.3%
            Address::zero(),
        );
        m.total_stake = Amount::new(10_000);
        m.status = MarketStatus::Resolved {
            winning_outcome: winning,
            accuracy_score: if winning == 0 { 1.5 } else { 2.5 },
        };
        m
    }

    fn add_position(ledger: &mut PositionLedger, user: Address, market_id: u64, outcome: usize, shares: f64) {
        let pos = ledger.get_or_create_mut(user, market_id, outcome);
        pos.add_shares(shares, Amount::ZERO);
    }

    #[test]
    fn test_binary_pro_rata_payout() {
        // 2 winners: user A holds 60 shares, user B holds 40 shares (total 100).
        // fee = 0.3% of 10_000 = 30; distributable = 9_970.
        // A gets 9970 * 60/100 = 5982; B gets 9970 * 40/100 = 3988.
        let market = resolved_binary(0);
        let mut ledger = PositionLedger::new();
        add_position(&mut ledger, addr(1), 1, 0, 60.0);
        add_position(&mut ledger, addr(2), 1, 0, 40.0);

        let result = PayoutEngine::compute_payouts(&market, &ledger).unwrap();
        assert_eq!(result.protocol_fee, Amount::new(30));

        let user_a = result.payouts.iter().find(|p| p.user == addr(1)).unwrap();
        let user_b = result.payouts.iter().find(|p| p.user == addr(2)).unwrap();

        assert_eq!(user_a.amount, Amount::new(5982));
        assert_eq!(user_b.amount, Amount::new(3988));
    }

    #[test]
    fn test_unresolved_market_returns_error() {
        let mut market = ForecastMarket::new_binary(
            99,
            [0u8; 32],
            "R",
            WeatherMetric::Wind,
            100,
            2.0,
            100.0,
            30,
            Address::zero(),
        );
        market.total_stake = Amount::new(1000);
        let ledger = PositionLedger::new();
        let err = PayoutEngine::compute_payouts(&market, &ledger).unwrap_err();
        assert!(matches!(err, PredictionError::NotResolved { .. }));
    }

    #[test]
    fn test_no_winners_returns_error() {
        let market = resolved_binary(0); // winning outcome 0 = Yes, but no positions
        let ledger = PositionLedger::new();
        let err = PayoutEngine::compute_payouts(&market, &ledger).unwrap_err();
        assert!(matches!(err, PredictionError::NoWinners));
    }

    #[test]
    fn test_scalar_payout_proportional_to_score() {
        // 2 participants with equal shares, score = 0.8.
        // fee = 30 bps of 10_000 = 30; distributable = 9_970.
        // each gets floor(9970 * 0.8 * 0.5) = floor(3988) = 3988.
        let mut market = ForecastMarket::new_scalar(
            2,
            [0u8; 32],
            "Pacific",
            WeatherMetric::Precipitation,
            50,
            100.0,
            30,
            Address::zero(),
        );
        market.total_stake = Amount::new(10_000);
        market.status = MarketStatus::Resolved {
            winning_outcome: 0,
            accuracy_score: 0.8,
        };

        let mut ledger = PositionLedger::new();
        add_position(&mut ledger, addr(1), 2, 0, 50.0);
        add_position(&mut ledger, addr(2), 2, 0, 50.0);

        let result = PayoutEngine::compute_payouts(&market, &ledger).unwrap();
        assert_eq!(result.protocol_fee, Amount::new(30));

        for p in &result.payouts {
            assert_eq!(p.amount, Amount::new(3988), "each winner should get 3988 units");
        }
    }

    #[test]
    fn test_scalar_payout_proportional_score_zero() {
        // score=0.0 → no payout to anyone → NoWinners
        let mut market = ForecastMarket::new_scalar(
            3,
            [0u8; 32],
            "Pacific",
            WeatherMetric::Precipitation,
            50,
            100.0,
            30,
            Address::zero(),
        );
        market.total_stake = Amount::new(10_000);
        market.status = MarketStatus::Resolved {
            winning_outcome: 0,
            accuracy_score: 0.0,
        };

        let mut ledger = PositionLedger::new();
        add_position(&mut ledger, addr(1), 3, 0, 50.0);
        let err = PayoutEngine::compute_payouts(&market, &ledger).unwrap_err();
        assert!(matches!(err, PredictionError::NoWinners));
    }
}
