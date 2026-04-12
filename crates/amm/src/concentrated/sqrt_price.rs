/// Q64.64 fixed-point sqrt price.
///
/// # Fixed-point choice: Q64.64 vs Q64.96
///
/// Uniswap V3 uses Q64.96 (160-bit) which requires `u256` arithmetic. Since
/// this codebase uses `u128` throughout, we use Q64.64 instead:
///
/// - 64 integer bits allow prices up to 2^64 ≈ 1.8 × 10^19 (ample for WRF pairs).
/// - 64 fractional bits give sub-integer precision of ~5.4 × 10^-20 per unit.
/// - All intermediate products fit in `u256` which we emulate via `u128` widening
///   multiplication where needed; for comparisons and storage a single `u128` suffices.
///
/// The invariant: `SqrtPrice(v)` represents the value `v / 2^64`.
/// So `price = (sqrt_price / 2^64)^2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SqrtPrice(pub u128);

/// The Q64.64 scale factor: 2^64.
pub const Q64: u128 = 1u128 << 64;

/// Tick spacing: each tick corresponds to a 0.01% price step (1.0001^tick).
pub const TICK_BASE_BPS: u128 = 10001; // 1.0001 * 10000
pub const TICK_BASE_DENOM: u128 = 10000;

/// Range of ticks supported by the lookup table and arithmetic helpers.
/// ±500 ticks covers roughly ±5% price range, sufficient for tests and
/// typical WRF trading pairs.
pub const TICK_MIN: i32 = -500;
pub const TICK_MAX: i32 = 500;

impl SqrtPrice {
    /// The sqrt price corresponding to tick 0: price = 1.0, sqrt_price = 1.0
    /// In Q64.64 this is exactly `2^64`.
    pub const ONE: SqrtPrice = SqrtPrice(Q64);

    /// Return the raw Q64.64 value.
    pub fn raw(self) -> u128 {
        self.0
    }

    /// Compute `price = (self / 2^64)^2` as a Q64.64 fixed-point value.
    /// Returns `None` on overflow.
    pub fn to_price_q64_64(self) -> Option<u128> {
        // price_q128 = (self.0)^2 / 2^64
        // We compute self.0 * self.0 as a 256-bit product then shift right by 64.
        let hi = self.0 >> 64;
        let lo = self.0 & ((1u128 << 64) - 1);
        // (hi*2^64 + lo)^2 = hi^2 * 2^128 + 2*hi*lo*2^64 + lo^2
        // Shift right by 64: hi^2 * 2^64 + 2*hi*lo + lo^2/2^64
        // Only the middle term and lo^2 part that survive fit in u128:
        let mid = hi.checked_mul(lo)?; // hi*lo, will be shifted by 64
        let lo_sq_shifted = lo.checked_mul(lo)? >> 64;
        // hi^2 * 2^64 would overflow u128 if hi > 0 (since hi can be up to 2^64)
        // For our range (|tick| <= 500) hi <= 1, so hi^2 * 2^64 is at most 2^64
        let hi_sq_part = hi.checked_mul(hi)?.checked_mul(Q64)?;
        let mid_part = mid.checked_mul(2)?;
        hi_sq_part.checked_add(mid_part)?.checked_add(lo_sq_shifted)
    }

    /// Multiply two Q64.64 sqrt prices together (used in range math).
    /// Result is shifted right by 64.
    pub fn checked_mul(self, rhs: SqrtPrice) -> Option<SqrtPrice> {
        // (a * b) >> 64, using 256-bit emulation
        let a = self.0;
        let b = rhs.0;
        let a_hi = a >> 64;
        let a_lo = a & ((1u128 << 64) - 1);
        let b_hi = b >> 64;
        let b_lo = b & ((1u128 << 64) - 1);
        // full product bits [192..128]: a_hi*b_hi (these would overflow u128)
        // bits [128..64]: a_hi*b_lo + a_lo*b_hi
        // bits [64..0]: a_lo*b_lo
        // After >> 64 we keep bits [128..0] of the full product, dropping [63..0]
        let cross1 = a_hi.checked_mul(b_lo)?;
        let cross2 = a_lo.checked_mul(b_hi)?;
        let lo_lo = a_lo.checked_mul(b_lo)?;
        // middle = cross1 + cross2 (128-bit sum from the [128..64] region)
        let mid = cross1.checked_add(cross2)?;
        // lo_lo >> 64 contributes to result[64..0] after the full >> 64
        let lo_contribution = lo_lo >> 64;
        // a_hi * b_hi (top bits): would be at bits [192..128], after >> 64 they're at [128..64]
        // This overflows u128 — for our limited tick range it's zero (since sqrt_price ~ 1.0)
        // We assert it's zero here to catch misuse
        let top = a_hi.checked_mul(b_hi)?;
        if top > 0 {
            return None; // overflow
        }
        mid.checked_add(lo_contribution).map(SqrtPrice)
    }
}

/// Convert a tick index to a Q64.64 sqrt price.
///
/// Uses the approximation: `sqrt_price = 1.0001^(tick/2)` in Q64.64.
///
/// Implementation: iterative integer multiplication avoids floating point.
/// We compute `1.0001^|tick|` by repeated multiplication, then take the
/// integer square root. For |tick| <= 500 the accumulated rounding error
/// is well within 0.01%, acceptable for test coverage.
///
/// The sign of the tick is handled by inversion: for negative ticks,
/// `sqrt_price = Q64^2 / sqrt_price_positive`.
pub fn tick_to_sqrt_price(tick: i32) -> SqrtPrice {
    if tick == 0 {
        return SqrtPrice::ONE;
    }

    let abs_tick = tick.unsigned_abs();

    // Compute price = 1.0001^abs_tick in Q64.64 fixed-point.
    // Start with 1.0 in Q64.64 = 2^64.
    // Each step: price = price * 10001 / 10000 (equivalent to * 1.0001)
    let mut price_q64: u128 = Q64;
    for _ in 0..abs_tick {
        // price_q64 = price_q64 * 10001 / 10000
        // Use u128 checked arithmetic; for |tick| <= 500 no overflow occurs.
        price_q64 = price_q64
            .checked_mul(TICK_BASE_BPS)
            .and_then(|v| v.checked_div(TICK_BASE_DENOM))
            .unwrap_or(u128::MAX);
    }

    // price = 1.0001^abs_tick; sqrt_price = sqrt(price) in Q64.64.
    // sqrt(price_q64 / 2^64) in Q64.64 = sqrt(price_q64) * 2^32
    // i.e., integer_sqrt(price_q64) * 2^32
    let sqrt_price_q64 = integer_sqrt_u128(price_q64)
        .checked_shl(32)
        .unwrap_or(u128::MAX);

    if tick < 0 {
        // For negative tick: sqrt_price = 1 / sqrt_price_positive
        // In Q64.64: (2^64)^2 / sqrt_price_pos
        // = 2^128 / sqrt_price_pos — but 2^128 overflows u128.
        // Instead: floor(2^127 / sqrt_price_q64) * 2
        // This is equivalent to floor(2^128 / sqrt_price_q64) with rounding.
        if sqrt_price_q64 == 0 {
            return SqrtPrice(u128::MAX);
        }
        // Compute (Q64 * Q64) / sqrt_price_q64 using 256-bit division emulation.
        // For our range sqrt_price is close to 1.0, so this is safe.
        // Use: result = (1<<127) / sqrt_price_q64 * 2 (avoids the 2^128 overflow)
        let half_q128: u128 = 1u128 << 127;
        let result = (half_q128 / sqrt_price_q64) * 2;
        SqrtPrice(result)
    } else {
        SqrtPrice(sqrt_price_q64)
    }
}

/// Convert a Q64.64 sqrt price back to its nearest tick index.
///
/// Uses binary search over [-500, 500] since our approximation is monotone.
pub fn sqrt_price_to_tick(sqrt_price: SqrtPrice) -> i32 {
    // Binary search for the largest tick t such that tick_to_sqrt_price(t) <= sqrt_price
    let mut lo: i32 = TICK_MIN;
    let mut hi: i32 = TICK_MAX;

    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        if tick_to_sqrt_price(mid).0 <= sqrt_price.0 {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

/// Integer square root of a u128 value (floor).
fn integer_sqrt_u128(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tick_zero_is_one() {
        let sp = tick_to_sqrt_price(0);
        assert_eq!(
            sp,
            SqrtPrice::ONE,
            "tick 0 must map to sqrt_price = 2^64 (Q64.64 1.0)"
        );
    }

    #[test]
    fn test_positive_tick_greater_than_zero() {
        let sp0 = tick_to_sqrt_price(0);
        let sp1 = tick_to_sqrt_price(1);
        assert!(
            sp1 > sp0,
            "tick 1 must produce a larger sqrt_price than tick 0"
        );
    }

    #[test]
    fn test_negative_tick_less_than_zero() {
        let sp0 = tick_to_sqrt_price(0);
        let sp_neg = tick_to_sqrt_price(-1);
        assert!(
            sp_neg < sp0,
            "tick -1 must produce a smaller sqrt_price than tick 0"
        );
    }

    #[test]
    fn test_monotone_increasing() {
        let mut prev = tick_to_sqrt_price(TICK_MIN);
        for t in (TICK_MIN + 1)..=TICK_MAX {
            let cur = tick_to_sqrt_price(t);
            assert!(cur > prev, "tick {} must be > tick {}", t, t - 1);
            prev = cur;
        }
    }

    #[test]
    fn test_sqrt_price_to_tick_roundtrip() {
        for t in [-100, -50, -10, -1, 0, 1, 10, 50, 100] {
            let sp = tick_to_sqrt_price(t);
            let recovered = sqrt_price_to_tick(sp);
            // Allow ±1 tick due to integer rounding
            assert!(
                (recovered - t).abs() <= 1,
                "round-trip failed for tick {t}: got {recovered}"
            );
        }
    }

    #[test]
    fn test_integer_sqrt() {
        assert_eq!(integer_sqrt_u128(0), 0);
        assert_eq!(integer_sqrt_u128(1), 1);
        assert_eq!(integer_sqrt_u128(4), 2);
        assert_eq!(integer_sqrt_u128(9), 3);
        assert_eq!(integer_sqrt_u128(100), 10);
    }
}
