//! Margin engine: initial/maintenance margin, mark-to-market, liquidation hook.
//!
//! Margin is held in WRF base units.  The engine tracks per-account balances and
//! evaluates positions against a mark price to determine under-margined accounts.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::types::{Address, Amount};
use crate::error::{DerivativesError, DerivativesResult};
use crate::futures::FutureId;

/// Configuration for margin requirements.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MarginConfig {
    /// Initial margin fraction in basis points (e.g., 1000 bps = 10%).
    pub initial_margin_bps: u64,
    /// Maintenance margin fraction in basis points (e.g., 500 bps = 5%).
    pub maintenance_margin_bps: u64,
}

impl MarginConfig {
    pub fn new(initial_margin_bps: u64, maintenance_margin_bps: u64) -> DerivativesResult<Self> {
        if maintenance_margin_bps >= initial_margin_bps {
            return Err(DerivativesError::InvalidParameter {
                reason: "maintenance_margin_bps must be < initial_margin_bps".into(),
            });
        }
        if initial_margin_bps > 10_000 {
            return Err(DerivativesError::InvalidParameter {
                reason: "initial_margin_bps must not exceed 10_000 (100%)".into(),
            });
        }
        Ok(Self {
            initial_margin_bps,
            maintenance_margin_bps,
        })
    }

    /// Default: 10% initial, 5% maintenance.
    pub fn default_config() -> Self {
        Self {
            initial_margin_bps: 1_000,
            maintenance_margin_bps: 500,
        }
    }

    /// Compute required initial margin for a notional value.
    /// `notional` is in WRF base units.
    pub fn initial_margin_required(&self, notional: Amount) -> Option<Amount> {
        notional.mul_fraction(self.initial_margin_bps as u128, 10_000)
    }

    /// Compute maintenance margin for a notional value.
    pub fn maintenance_margin_required(&self, notional: Amount) -> Option<Amount> {
        notional.mul_fraction(self.maintenance_margin_bps as u128, 10_000)
    }
}

/// A single margin account for one trader.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarginAccount {
    pub trader: Address,
    /// Total deposited margin in WRF base units.
    pub balance: Amount,
    /// Reserved margin (locked against open positions).
    pub reserved: Amount,
}

impl MarginAccount {
    pub fn new(trader: Address) -> Self {
        Self {
            trader,
            balance: Amount::ZERO,
            reserved: Amount::ZERO,
        }
    }

    pub fn free_balance(&self) -> Amount {
        self.balance
            .checked_sub(self.reserved)
            .unwrap_or(Amount::ZERO)
    }
}

/// Callback interface for margin liquidation events.
pub trait LiquidationHook {
    /// Called when an account drops below maintenance margin.
    /// The hook should record/emit a liquidation event and can modify external state.
    fn on_under_margin(&mut self, trader: Address, shortfall: Amount, future_id: FutureId);
}

/// A no-op liquidation hook (useful in tests / as a default).
#[derive(Debug, Default)]
pub struct NoOpLiquidationHook;

impl LiquidationHook for NoOpLiquidationHook {
    fn on_under_margin(&mut self, _trader: Address, _shortfall: Amount, _future_id: FutureId) {}
}

/// Collects liquidation events for test inspection.
#[derive(Debug, Default)]
pub struct RecordingLiquidationHook {
    pub events: Vec<(Address, Amount, FutureId)>,
}

impl LiquidationHook for RecordingLiquidationHook {
    fn on_under_margin(&mut self, trader: Address, shortfall: Amount, future_id: FutureId) {
        self.events.push((trader, shortfall, future_id));
    }
}

/// Manages margin accounts for all traders.
#[derive(Debug)]
pub struct MarginEngine {
    accounts: HashMap<Address, MarginAccount>,
    config: MarginConfig,
}

impl MarginEngine {
    pub fn new(config: MarginConfig) -> Self {
        Self {
            accounts: HashMap::new(),
            config,
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(MarginConfig::default_config())
    }

    /// Deposit margin into an account.
    pub fn deposit(&mut self, trader: Address, amount: Amount) -> DerivativesResult<()> {
        if amount.is_zero() {
            return Err(DerivativesError::InvalidParameter {
                reason: "deposit amount must be non-zero".into(),
            });
        }
        let account = self.accounts.entry(trader).or_insert_with(|| MarginAccount::new(trader));
        account.balance = account
            .balance
            .checked_add(amount)
            .ok_or(DerivativesError::Overflow {
                operation: "margin deposit".into(),
            })?;
        Ok(())
    }

    /// Withdraw free (unreserved) margin.
    pub fn withdraw(&mut self, trader: Address, amount: Amount) -> DerivativesResult<()> {
        let account = self
            .accounts
            .get_mut(&trader)
            .ok_or(DerivativesError::InsufficientMargin {
                required: amount,
                available: Amount::ZERO,
            })?;

        let free = account.free_balance();
        if amount > free {
            return Err(DerivativesError::InsufficientMargin {
                required: amount,
                available: free,
            });
        }
        account.balance = account.balance.checked_sub(amount).ok_or(DerivativesError::Overflow {
            operation: "margin withdraw".into(),
        })?;
        Ok(())
    }

    /// Reserve initial margin for a new position.
    /// `notional` = contracts × current_price_scaled / 1_000_000 × tick_size
    pub fn reserve_initial_margin(
        &mut self,
        trader: Address,
        notional: Amount,
    ) -> DerivativesResult<()> {
        let required = self
            .config
            .initial_margin_required(notional)
            .ok_or(DerivativesError::Overflow {
                operation: "initial margin calculation".into(),
            })?;

        let account = self
            .accounts
            .get_mut(&trader)
            .ok_or(DerivativesError::InsufficientMargin {
                required,
                available: Amount::ZERO,
            })?;

        if required > account.free_balance() {
            return Err(DerivativesError::InsufficientMargin {
                required,
                available: account.free_balance(),
            });
        }

        account.reserved = account
            .reserved
            .checked_add(required)
            .ok_or(DerivativesError::Overflow {
                operation: "reserve margin".into(),
            })?;
        Ok(())
    }

    /// Release reserved margin (e.g., on position close or settlement).
    pub fn release_reserved(&mut self, trader: Address, amount: Amount) -> DerivativesResult<()> {
        let account = self.accounts.get_mut(&trader).ok_or_else(|| {
            DerivativesError::InvalidParameter {
                reason: "account not found".into(),
            }
        })?;
        account.reserved = account.reserved.checked_sub(amount).ok_or(
            DerivativesError::Overflow {
                operation: "release reserved margin".into(),
            },
        )?;
        Ok(())
    }

    /// Mark-to-market: apply unrealised P&L adjustment to an account's balance.
    /// `pnl_signed` is positive (gain) or negative (loss) in WRF base units.
    pub fn mark_to_market(&mut self, trader: Address, pnl_signed: i128) -> DerivativesResult<()> {
        let account = self
            .accounts
            .entry(trader)
            .or_insert_with(|| MarginAccount::new(trader));

        if pnl_signed >= 0 {
            let gain = Amount::new(pnl_signed as u128);
            account.balance = account
                .balance
                .checked_add(gain)
                .ok_or(DerivativesError::Overflow {
                    operation: "mtm gain".into(),
                })?;
        } else {
            let loss = Amount::new((-pnl_signed) as u128);
            // If loss exceeds balance, clamp to zero (account is insolvent).
            account.balance = account.balance.checked_sub(loss).unwrap_or(Amount::ZERO);
        }
        Ok(())
    }

    /// Check all accounts against maintenance margin for a given position's notional.
    /// Fires the liquidation hook for any under-margined accounts.
    ///
    /// Returns the list of under-margined traders.
    pub fn check_maintenance<H: LiquidationHook>(
        &self,
        future_id: FutureId,
        notionals: &HashMap<Address, Amount>,
        hook: &mut H,
    ) -> Vec<Address> {
        let mut under_margined = Vec::new();
        for (trader, &notional) in notionals {
            let required = match self.config.maintenance_margin_required(notional) {
                Some(r) => r,
                None => continue,
            };
            let account = match self.accounts.get(trader) {
                Some(a) => a,
                None => {
                    hook.on_under_margin(*trader, required, future_id);
                    under_margined.push(*trader);
                    continue;
                }
            };
            if account.balance < required {
                let shortfall = required.checked_sub(account.balance).unwrap_or(required);
                hook.on_under_margin(*trader, shortfall, future_id);
                under_margined.push(*trader);
            }
        }
        under_margined
    }

    pub fn get_account(&self, trader: &Address) -> Option<&MarginAccount> {
        self.accounts.get(trader)
    }

    pub fn config(&self) -> &MarginConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: u8) -> Address {
        let mut b = [0u8; 32];
        b[31] = n;
        Address::from_bytes(b)
    }

    #[test]
    fn test_deposit_and_balance() {
        let mut engine = MarginEngine::with_default_config();
        engine.deposit(addr(1), Amount::new(10_000)).unwrap();
        assert_eq!(engine.get_account(&addr(1)).unwrap().balance, Amount::new(10_000));
    }

    #[test]
    fn test_withdraw_within_free_balance() {
        let mut engine = MarginEngine::with_default_config();
        engine.deposit(addr(1), Amount::new(10_000)).unwrap();
        engine.withdraw(addr(1), Amount::new(3_000)).unwrap();
        assert_eq!(engine.get_account(&addr(1)).unwrap().balance, Amount::new(7_000));
    }

    #[test]
    fn test_withdraw_exceeds_free_fails() {
        let mut engine = MarginEngine::with_default_config();
        engine.deposit(addr(1), Amount::new(1_000)).unwrap();
        assert!(engine.withdraw(addr(1), Amount::new(2_000)).is_err());
    }

    #[test]
    fn test_reserve_initial_margin() {
        let mut engine = MarginEngine::with_default_config();
        // Config: 10% initial. notional = 10_000 → required = 1_000
        engine.deposit(addr(1), Amount::new(2_000)).unwrap();
        engine.reserve_initial_margin(addr(1), Amount::new(10_000)).unwrap();
        let acc = engine.get_account(&addr(1)).unwrap();
        assert_eq!(acc.reserved, Amount::new(1_000));
        assert_eq!(acc.free_balance(), Amount::new(1_000));
    }

    #[test]
    fn test_reserve_exceeds_free_fails() {
        let mut engine = MarginEngine::with_default_config();
        engine.deposit(addr(1), Amount::new(500)).unwrap();
        // initial margin for 10_000 notional = 1_000, but we only have 500 free
        assert!(engine.reserve_initial_margin(addr(1), Amount::new(10_000)).is_err());
    }

    #[test]
    fn test_mark_to_market_gain() {
        let mut engine = MarginEngine::with_default_config();
        engine.deposit(addr(1), Amount::new(1_000)).unwrap();
        engine.mark_to_market(addr(1), 500).unwrap();
        assert_eq!(engine.get_account(&addr(1)).unwrap().balance, Amount::new(1_500));
    }

    #[test]
    fn test_mark_to_market_loss() {
        let mut engine = MarginEngine::with_default_config();
        engine.deposit(addr(1), Amount::new(1_000)).unwrap();
        engine.mark_to_market(addr(1), -400).unwrap();
        assert_eq!(engine.get_account(&addr(1)).unwrap().balance, Amount::new(600));
    }

    #[test]
    fn test_mark_to_market_loss_clamps_to_zero() {
        let mut engine = MarginEngine::with_default_config();
        engine.deposit(addr(1), Amount::new(100)).unwrap();
        engine.mark_to_market(addr(1), -500).unwrap();
        // balance should clamp to 0, not underflow
        assert_eq!(engine.get_account(&addr(1)).unwrap().balance, Amount::ZERO);
    }

    #[test]
    fn test_under_margined_triggers_liquidation_hook() {
        let mut engine = MarginEngine::with_default_config();
        // Config: maintenance margin = 5% of notional.
        // Trader has 10 WRF but notional = 10_000 → maintenance required = 500 WRF
        // Balance 10 < 500 → under-margined.
        engine.deposit(addr(1), Amount::new(10)).unwrap();

        let mut notionals = HashMap::new();
        notionals.insert(addr(1), Amount::new(10_000));

        let mut hook = RecordingLiquidationHook::default();
        let under = engine.check_maintenance(FutureId(42), &notionals, &mut hook);

        assert_eq!(under.len(), 1);
        assert_eq!(under[0], addr(1));
        assert_eq!(hook.events.len(), 1);
        let (trader, shortfall, fid) = hook.events[0];
        assert_eq!(trader, addr(1));
        // shortfall = 500 - 10 = 490
        assert_eq!(shortfall, Amount::new(490));
        assert_eq!(fid, FutureId(42));
    }

    #[test]
    fn test_well_margined_does_not_trigger_hook() {
        let mut engine = MarginEngine::with_default_config();
        // maintenance = 5% of 1_000 = 50; balance = 1_000 → well margined
        engine.deposit(addr(1), Amount::new(1_000)).unwrap();
        let mut notionals = HashMap::new();
        notionals.insert(addr(1), Amount::new(1_000));
        let mut hook = RecordingLiquidationHook::default();
        let under = engine.check_maintenance(FutureId(1), &notionals, &mut hook);
        assert!(under.is_empty());
        assert!(hook.events.is_empty());
    }
}
