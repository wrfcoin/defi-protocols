use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::types::{Address, Amount};

/// A user's position in a single outcome of a single market.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserPosition {
    pub user: Address,
    pub market_id: u64,
    pub outcome_index: usize,
    /// Number of LMSR shares held (f64 — mirrors LMSR share accounting).
    pub shares: f64,
    /// Total WRF base units spent to acquire these shares (for record-keeping).
    pub cost_basis: Amount,
}

impl UserPosition {
    pub fn new(user: Address, market_id: u64, outcome_index: usize) -> Self {
        Self {
            user,
            market_id,
            outcome_index,
            shares: 0.0,
            cost_basis: Amount::ZERO,
        }
    }

    pub fn add_shares(&mut self, delta: f64, cost: Amount) {
        self.shares += delta;
        self.cost_basis = self
            .cost_basis
            .checked_add(cost)
            .unwrap_or(self.cost_basis);
    }

    pub fn remove_shares(&mut self, delta: f64) {
        self.shares = f64::max(0.0, self.shares - delta);
    }
}

/// Key for the position ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PositionKey {
    pub user: Address,
    pub market_id: u64,
    pub outcome_index: usize,
}

/// In-memory ledger of all user positions across all markets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PositionLedger {
    #[serde(skip)]
    positions: HashMap<(u64, usize, [u8; 32]), UserPosition>,
}

impl PositionLedger {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(market_id: u64, outcome_index: usize, user: &Address) -> (u64, usize, [u8; 32]) {
        (market_id, outcome_index, user.0)
    }

    /// Get or create a mutable position.
    pub fn get_or_create_mut(
        &mut self,
        user: Address,
        market_id: u64,
        outcome_index: usize,
    ) -> &mut UserPosition {
        let k = Self::key(market_id, outcome_index, &user);
        self.positions
            .entry(k)
            .or_insert_with(|| UserPosition::new(user, market_id, outcome_index))
    }

    /// Get an immutable reference to a position (None if no position exists).
    pub fn get(
        &self,
        user: &Address,
        market_id: u64,
        outcome_index: usize,
    ) -> Option<&UserPosition> {
        let k = Self::key(market_id, outcome_index, user);
        self.positions.get(&k)
    }

    /// Return all positions for a given market, across all outcomes and users.
    pub fn positions_for_market(&self, market_id: u64) -> Vec<&UserPosition> {
        self.positions
            .values()
            .filter(|p| p.market_id == market_id)
            .collect()
    }

    /// Return all positions for a given market and outcome index.
    pub fn positions_for_outcome(
        &self,
        market_id: u64,
        outcome_index: usize,
    ) -> Vec<&UserPosition> {
        self.positions
            .values()
            .filter(|p| p.market_id == market_id && p.outcome_index == outcome_index)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_address(byte: u8) -> Address {
        Address([byte; 32])
    }

    #[test]
    fn test_remove_shares_zero_floor() {
        let mut pos = UserPosition::new(dummy_address(1), 0, 0);
        pos.shares = 5.0;
        // Remove more than available — should clamp to 0, not go negative.
        pos.remove_shares(10.0);
        assert_eq!(pos.shares, 0.0, "shares must not go below zero");
    }
}
