use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Amount, AssetId, Timestamp};

use crate::pool::LendingPool;

/// Default flash loan fee in basis points (0.09%, like Aave).
pub const DEFAULT_FLASH_LOAN_FEE_BPS: u64 = 9;

/// Execute a flash loan against a lending pool.
///
/// Temporarily withdraws `amount` from the pool, passes it to the callback,
/// and expects the callback to return at least `amount + fee`. The returned
/// funds (principal + fee) are deposited back into the pool.
///
/// # Arguments
/// * `pool` - The lending pool to borrow from.
/// * `asset` - The asset to flash-borrow.
/// * `amount` - The amount to borrow.
/// * `fee_bps` - Fee in basis points charged on the borrowed amount.
/// * `timestamp` - Current timestamp for pool bookkeeping.
/// * `callback` - User logic that receives the borrowed amount and must return
///   at least `amount + fee`.
///
/// # Returns
/// The fee collected on success.
pub fn execute_flash_loan<F>(
    pool: &mut LendingPool,
    asset: &AssetId,
    amount: Amount,
    fee_bps: u64,
    timestamp: Timestamp,
    callback: F,
) -> DefiResult<Amount>
where
    F: FnOnce(Amount) -> DefiResult<Amount>,
{
    if amount.is_zero() {
        return Err(DefiError::InvalidAmount {
            reason: "flash loan amount must be non-zero".into(),
        });
    }

    // Calculate the required fee.
    let fee = amount
        .mul_fraction(fee_bps as u128, 10_000)
        .ok_or(DefiError::Overflow {
            operation: "flash_loan fee".into(),
        })?;

    let required_return = amount.checked_add(fee).ok_or(DefiError::Overflow {
        operation: "flash_loan required return".into(),
    })?;

    // Verify sufficient liquidity before proceeding.
    let available = pool.available_liquidity(asset)?;
    if amount > available {
        return Err(DefiError::InsufficientLiquidity {
            requested: amount,
            available,
        });
    }

    // Withdraw the loan amount from the pool.
    pool.withdraw(asset, amount, timestamp)?;

    // Execute user logic.
    let returned = callback(amount)?;

    // Verify the callback returned enough.
    if returned < required_return {
        // Re-deposit what was returned to avoid leaving the pool short.
        if !returned.is_zero() {
            pool.deposit(asset, returned, timestamp)?;
        }
        return Err(DefiError::InvalidAmount {
            reason: format!(
                "flash loan underpayment: returned {}, required {}",
                returned, required_return
            ),
        });
    }

    // Deposit the full returned amount back into the pool (principal + fee).
    pool.deposit(asset, returned, timestamp)?;

    Ok(fee)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrfcoin_defi_common::types::Timestamp;

    fn ts(secs: u64) -> Timestamp {
        Timestamp::new(secs)
    }

    fn asset(name: &str) -> AssetId {
        AssetId::new(name)
    }

    fn setup_pool() -> LendingPool {
        let mut pool = LendingPool::new();
        pool.init_reserve(asset("WRF"), ts(0));
        pool.deposit(&asset("WRF"), Amount::new(10_000), ts(1)).unwrap();
        pool
    }

    #[test]
    fn test_flash_loan_success() {
        let mut pool = setup_pool();
        let fee = execute_flash_loan(
            &mut pool,
            &asset("WRF"),
            Amount::new(1_000),
            DEFAULT_FLASH_LOAN_FEE_BPS,
            ts(10),
            |borrowed| {
                assert_eq!(borrowed, Amount::new(1_000));
                // Simulate profitable arbitrage: return principal + fee.
                // fee = 1000 * 9 / 10000 = 0 (truncated, so need to return at least 1000)
                Ok(Amount::new(1_001))
            },
        )
        .unwrap();

        // Fee = 1000 * 9 / 10000 = 0 (integer truncation at small amounts)
        assert_eq!(fee, Amount::new(0));
        // Pool gained the extra 1 from the callback (10_000 - 1_000 + 1_001 = 10_001)
        assert_eq!(
            pool.reserves[&asset("WRF")].total_deposits,
            Amount::new(10_001)
        );
    }

    #[test]
    fn test_flash_loan_with_meaningful_fee() {
        let mut pool = setup_pool();
        let loan_amount = Amount::new(100_000_0); // 1M units
        // First add more liquidity
        pool.deposit(&asset("WRF"), Amount::new(990_000_0), ts(2)).unwrap();
        // Pool now has 10M

        let fee = execute_flash_loan(
            &mut pool,
            &asset("WRF"),
            loan_amount,
            DEFAULT_FLASH_LOAN_FEE_BPS,
            ts(10),
            |_borrowed| {
                // fee = 1_000_000 * 9 / 10_000 = 900
                Ok(Amount::new(100_000_0 + 900))
            },
        )
        .unwrap();

        assert_eq!(fee, Amount::new(900));
    }

    #[test]
    fn test_flash_loan_underpayment_fails() {
        let mut pool = setup_pool();
        let result = execute_flash_loan(
            &mut pool,
            &asset("WRF"),
            Amount::new(1_000),
            100, // 1% fee = 10
            ts(10),
            |_| {
                // Return only the principal, not principal + fee.
                Ok(Amount::new(1_000))
            },
        );
        assert!(result.is_err());
        // Pool should have the returned 1000 re-deposited (9000 + 1000 = 10000)
        assert_eq!(
            pool.reserves[&asset("WRF")].total_deposits,
            Amount::new(10_000)
        );
    }

    #[test]
    fn test_flash_loan_insufficient_liquidity() {
        let mut pool = setup_pool();
        let result = execute_flash_loan(
            &mut pool,
            &asset("WRF"),
            Amount::new(20_000), // More than the 10_000 available
            DEFAULT_FLASH_LOAN_FEE_BPS,
            ts(10),
            |_| Ok(Amount::new(20_000)),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_flash_loan_zero_amount_fails() {
        let mut pool = setup_pool();
        let result = execute_flash_loan(
            &mut pool,
            &asset("WRF"),
            Amount::ZERO,
            DEFAULT_FLASH_LOAN_FEE_BPS,
            ts(10),
            |_| Ok(Amount::ZERO),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_flash_loan_callback_error_propagates() {
        let mut pool = setup_pool();
        let result = execute_flash_loan(
            &mut pool,
            &asset("WRF"),
            Amount::new(1_000),
            DEFAULT_FLASH_LOAN_FEE_BPS,
            ts(10),
            |_| {
                Err(DefiError::InvalidAmount {
                    reason: "callback failed".into(),
                })
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_flash_loan_custom_fee() {
        let mut pool = setup_pool();
        let fee = execute_flash_loan(
            &mut pool,
            &asset("WRF"),
            Amount::new(10_000),
            500, // 5% fee = 500
            ts(10),
            |_| Ok(Amount::new(10_500)),
        )
        .unwrap();

        assert_eq!(fee, Amount::new(500));
        // Pool: 10_000 - 10_000 + 10_500 = 10_500
        assert_eq!(
            pool.reserves[&asset("WRF")].total_deposits,
            Amount::new(10_500)
        );
    }
}
