use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Address, Amount, AssetId};

pub const BPS_SCALE: u64 = 10_000;
pub const DEFAULT_ACCESS_DURATION_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GeographicRegion(pub String);

impl GeographicRegion {
    pub fn new(code: impl Into<String>) -> DefiResult<Self> {
        let code = code.into();
        if code.trim().is_empty() {
            return Err(DefiError::InvalidAmount {
                reason: "geographic region must be non-empty".into(),
            });
        }
        Ok(Self(code))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataTier {
    Raw,
    Validated,
    Forecast,
    Premium,
}

impl DataTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            DataTier::Raw => "raw",
            DataTier::Validated => "validated",
            DataTier::Forecast => "forecast",
            DataTier::Premium => "premium",
        }
    }
}

/// Guardrails that mirror the #47 emission hardening pattern for subscription pricing:
/// cap outstanding claims and cap the marginal price growth multiplier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PricingGuardrails {
    pub max_outstanding_supply: Amount,
    pub max_price_multiplier_bps: u64,
}

impl Default for PricingGuardrails {
    fn default() -> Self {
        Self {
            max_outstanding_supply: Amount::new(10_000),
            max_price_multiplier_bps: 50_000,
        }
    }
}

impl PricingGuardrails {
    pub fn validate(&self) -> DefiResult<()> {
        if self.max_outstanding_supply.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "max_outstanding_supply must be non-zero".into(),
            });
        }
        if self.max_price_multiplier_bps < BPS_SCALE {
            return Err(DefiError::InvalidAmount {
                reason: "max_price_multiplier_bps must be at least 10_000".into(),
            });
        }
        Ok(())
    }

    fn capped_price(&self, base_price: Amount, uncapped_price: Amount) -> DefiResult<Amount> {
        let max_price = base_price
            .mul_fraction(self.max_price_multiplier_bps as u128, BPS_SCALE as u128)
            .ok_or(DefiError::Overflow {
                operation: "bonding curve max price cap".into(),
            })?;
        Ok(Amount::new(uncapped_price.0.min(max_price.0)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BondingCurve {
    Linear {
        base_price: Amount,
        slope: Amount,
    },
    Polynomial {
        base_price: Amount,
        coefficient: Amount,
        exponent: u8,
    },
    Exponential {
        base_price: Amount,
        growth_bps: u64,
    },
}

impl BondingCurve {
    pub fn validate(&self) -> DefiResult<()> {
        match self {
            BondingCurve::Linear { base_price, .. }
            | BondingCurve::Polynomial { base_price, .. }
            | BondingCurve::Exponential { base_price, .. } => {
                if base_price.is_zero() {
                    return Err(DefiError::InvalidAmount {
                        reason: "base_price must be non-zero".into(),
                    });
                }
            }
        }

        match self {
            BondingCurve::Linear { .. } => Ok(()),
            BondingCurve::Polynomial { exponent, .. } => {
                if *exponent == 0 {
                    return Err(DefiError::InvalidAmount {
                        reason: "polynomial exponent must be >= 1".into(),
                    });
                }
                Ok(())
            }
            BondingCurve::Exponential { growth_bps, .. } => {
                if *growth_bps == 0 {
                    return Err(DefiError::InvalidAmount {
                        reason: "exponential growth_bps must be non-zero".into(),
                    });
                }
                Ok(())
            }
        }
    }

    pub fn base_price(&self) -> Amount {
        match self {
            BondingCurve::Linear { base_price, .. }
            | BondingCurve::Polynomial { base_price, .. }
            | BondingCurve::Exponential { base_price, .. } => *base_price,
        }
    }

    pub fn marginal_price(
        &self,
        supply_before_purchase: Amount,
        guardrails: &PricingGuardrails,
    ) -> DefiResult<Amount> {
        self.validate()?;
        guardrails.validate()?;

        let uncapped_price = match self {
            BondingCurve::Linear { base_price, slope } => {
                let increment = slope.checked_mul(supply_before_purchase.0).ok_or(
                    DefiError::Overflow {
                        operation: "linear bonding curve increment".into(),
                    },
                )?;
                base_price.checked_add(increment).ok_or(DefiError::Overflow {
                    operation: "linear bonding curve marginal price".into(),
                })?
            }
            BondingCurve::Polynomial {
                base_price,
                coefficient,
                exponent,
            } => {
                let power = checked_pow(supply_before_purchase.0, *exponent as u32)?;
                let increment = coefficient.checked_mul(power).ok_or(DefiError::Overflow {
                    operation: "polynomial bonding curve increment".into(),
                })?;
                base_price.checked_add(increment).ok_or(DefiError::Overflow {
                    operation: "polynomial bonding curve marginal price".into(),
                })?
            }
            BondingCurve::Exponential {
                base_price,
                growth_bps,
            } => {
                let mut price = *base_price;
                for _ in 0..u128_to_usize(supply_before_purchase.0)? {
                    price = price
                        .mul_fraction(
                            (BPS_SCALE + growth_bps) as u128,
                            BPS_SCALE as u128,
                        )
                        .ok_or(DefiError::Overflow {
                            operation: "exponential bonding curve marginal price".into(),
                        })?;
                }
                price
            }
        };

        guardrails.capped_price(self.base_price(), uncapped_price)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurchaseQuote {
    pub tokens_minted: Amount,
    pub total_spent: Amount,
    pub reserve_added: Amount,
    pub contributor_funding: Amount,
    pub unspent_budget: Amount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaleQuote {
    pub tokens_burned: Amount,
    pub gross_curve_value: Amount,
    pub reserve_refund: Amount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionToken {
    pub token_id: AssetId,
    pub region: GeographicRegion,
    pub data_tier: DataTier,
    pub curve: BondingCurve,
    pub supply: Amount,
    pub reserve: Amount,
    pub contributor_funding: Amount,
    pub total_purchase_volume: Amount,
    pub total_refunds: Amount,
    pub reserve_ratio_bps: u64,
    pub access_duration_secs: u64,
    pub guardrails: PricingGuardrails,
    balances: HashMap<Address, Amount>,
}

impl SubscriptionToken {
    pub fn new(
        region: GeographicRegion,
        data_tier: DataTier,
        curve: BondingCurve,
        reserve_ratio_bps: u64,
        access_duration_secs: u64,
        guardrails: PricingGuardrails,
    ) -> DefiResult<Self> {
        if reserve_ratio_bps == 0 || reserve_ratio_bps > BPS_SCALE {
            return Err(DefiError::InvalidAmount {
                reason: "reserve_ratio_bps must be within 1..=10_000".into(),
            });
        }
        if access_duration_secs == 0 {
            return Err(DefiError::InvalidAmount {
                reason: "access_duration_secs must be non-zero".into(),
            });
        }
        curve.validate()?;
        guardrails.validate()?;

        Ok(Self {
            token_id: AssetId::new(format!(
                "weather-subscription:{}:{}",
                region.as_str(),
                data_tier.as_str()
            )),
            region,
            data_tier,
            curve,
            supply: Amount::ZERO,
            reserve: Amount::ZERO,
            contributor_funding: Amount::ZERO,
            total_purchase_volume: Amount::ZERO,
            total_refunds: Amount::ZERO,
            reserve_ratio_bps,
            access_duration_secs,
            guardrails,
            balances: HashMap::new(),
        })
    }

    pub fn balance_of(&self, owner: &Address) -> Amount {
        self.balances.get(owner).copied().unwrap_or(Amount::ZERO)
    }

    pub fn price_for_next_token(&self) -> DefiResult<Amount> {
        self.curve.marginal_price(self.supply, &self.guardrails)
    }

    pub fn quote_buy(&self, wrf_budget: Amount) -> DefiResult<PurchaseQuote> {
        if wrf_budget.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "wrf_budget must be non-zero".into(),
            });
        }

        if self.supply >= self.guardrails.max_outstanding_supply {
            return Err(DefiError::InvalidAmount {
                reason: "subscription supply cap already reached".into(),
            });
        }

        let mut simulated_supply = self.supply;
        let mut spent = Amount::ZERO;
        let mut reserve_added = Amount::ZERO;
        let mut contributor_funding = Amount::ZERO;
        let mut minted = Amount::ZERO;

        while simulated_supply < self.guardrails.max_outstanding_supply {
            let price = self
                .curve
                .marginal_price(simulated_supply, &self.guardrails)?;
            let next_spent = spent.checked_add(price).ok_or(DefiError::Overflow {
                operation: "bonding curve quote_buy spend".into(),
            })?;
            if next_spent > wrf_budget {
                break;
            }

            let reserve_piece =
                reserve_backed_component(price, self.reserve_ratio_bps)?;
            let contributor_piece = price
                .checked_sub(reserve_piece)
                .ok_or(DefiError::Overflow {
                    operation: "bonding curve quote_buy contributor split".into(),
                })?;

            spent = next_spent;
            reserve_added = reserve_added
                .checked_add(reserve_piece)
                .ok_or(DefiError::Overflow {
                    operation: "bonding curve quote_buy reserve".into(),
                })?;
            contributor_funding = contributor_funding
                .checked_add(contributor_piece)
                .ok_or(DefiError::Overflow {
                    operation: "bonding curve quote_buy contributor".into(),
                })?;
            minted = minted.checked_add(Amount::new(1)).ok_or(DefiError::Overflow {
                operation: "bonding curve quote_buy mint count".into(),
            })?;
            simulated_supply = simulated_supply
                .checked_add(Amount::new(1))
                .ok_or(DefiError::Overflow {
                    operation: "bonding curve quote_buy supply".into(),
                })?;
        }

        if minted.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "budget is insufficient to purchase the next subscription token".into(),
            });
        }

        let unspent_budget = wrf_budget.checked_sub(spent).ok_or(DefiError::Overflow {
            operation: "bonding curve quote_buy unspent".into(),
        })?;

        Ok(PurchaseQuote {
            tokens_minted: minted,
            total_spent: spent,
            reserve_added,
            contributor_funding,
            unspent_budget,
        })
    }

    pub fn buy(&mut self, buyer: Address, wrf_budget: Amount) -> DefiResult<PurchaseQuote> {
        let quote = self.quote_buy(wrf_budget)?;

        self.supply = self
            .supply
            .checked_add(quote.tokens_minted)
            .ok_or(DefiError::Overflow {
                operation: "bonding curve buy supply".into(),
            })?;
        self.reserve = self
            .reserve
            .checked_add(quote.reserve_added)
            .ok_or(DefiError::Overflow {
                operation: "bonding curve buy reserve".into(),
            })?;
        self.contributor_funding = self
            .contributor_funding
            .checked_add(quote.contributor_funding)
            .ok_or(DefiError::Overflow {
                operation: "bonding curve buy contributor funding".into(),
            })?;
        self.total_purchase_volume = self
            .total_purchase_volume
            .checked_add(quote.total_spent)
            .ok_or(DefiError::Overflow {
                operation: "bonding curve buy purchase volume".into(),
            })?;

        let balance = self.balances.entry(buyer).or_insert(Amount::ZERO);
        *balance = balance
            .checked_add(quote.tokens_minted)
            .ok_or(DefiError::Overflow {
                operation: "bonding curve buy balance".into(),
            })?;

        Ok(quote)
    }

    pub fn quote_sell(&self, token_amount: Amount) -> DefiResult<SaleQuote> {
        if token_amount.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "token_amount must be non-zero".into(),
            });
        }
        if token_amount > self.supply {
            return Err(DefiError::InsufficientBalance {
                required: token_amount,
                available: self.supply,
            });
        }

        let mut simulated_supply = self.supply;
        let mut gross_curve_value = Amount::ZERO;
        let mut reserve_refund = Amount::ZERO;

        for _ in 0..u128_to_usize(token_amount.0)? {
            simulated_supply = simulated_supply
                .checked_sub(Amount::new(1))
                .ok_or(DefiError::Overflow {
                    operation: "bonding curve quote_sell supply decrement".into(),
                })?;
            let price = self
                .curve
                .marginal_price(simulated_supply, &self.guardrails)?;
            let reserve_piece =
                reserve_backed_component(price, self.reserve_ratio_bps)?;

            gross_curve_value = gross_curve_value
                .checked_add(price)
                .ok_or(DefiError::Overflow {
                    operation: "bonding curve quote_sell gross value".into(),
                })?;
            reserve_refund = reserve_refund
                .checked_add(reserve_piece)
                .ok_or(DefiError::Overflow {
                    operation: "bonding curve quote_sell reserve refund".into(),
                })?;
        }

        if reserve_refund > self.reserve {
            return Err(DefiError::InsufficientReservedBalance {
                required: reserve_refund,
                available: self.reserve,
            });
        }

        Ok(SaleQuote {
            tokens_burned: token_amount,
            gross_curve_value,
            reserve_refund,
        })
    }

    pub fn sell(&mut self, seller: Address, token_amount: Amount) -> DefiResult<SaleQuote> {
        let balance = self.balances.get(&seller).copied().unwrap_or(Amount::ZERO);
        if balance < token_amount {
            return Err(DefiError::InsufficientBalance {
                required: token_amount,
                available: balance,
            });
        }

        let quote = self.quote_sell(token_amount)?;

        let updated_balance = balance.checked_sub(token_amount).ok_or(DefiError::Overflow {
            operation: "bonding curve sell balance".into(),
        })?;
        if updated_balance.is_zero() {
            self.balances.remove(&seller);
        } else {
            self.balances.insert(seller, updated_balance);
        }

        self.supply = self
            .supply
            .checked_sub(token_amount)
            .ok_or(DefiError::Overflow {
                operation: "bonding curve sell supply".into(),
            })?;
        self.reserve = self
            .reserve
            .checked_sub(quote.reserve_refund)
            .ok_or(DefiError::Overflow {
                operation: "bonding curve sell reserve".into(),
            })?;
        self.total_refunds = self
            .total_refunds
            .checked_add(quote.reserve_refund)
            .ok_or(DefiError::Overflow {
                operation: "bonding curve sell refund volume".into(),
            })?;

        Ok(quote)
    }
}

fn reserve_backed_component(price: Amount, reserve_ratio_bps: u64) -> DefiResult<Amount> {
    price
        .mul_fraction(reserve_ratio_bps as u128, BPS_SCALE as u128)
        .ok_or(DefiError::Overflow {
            operation: "bonding curve reserve-backed component".into(),
        })
}

fn checked_pow(mut base: u128, mut exponent: u32) -> DefiResult<u128> {
    let mut result = 1u128;
    while exponent > 0 {
        if exponent & 1 == 1 {
            result = result.checked_mul(base).ok_or(DefiError::Overflow {
                operation: "bonding curve checked_pow multiply result".into(),
            })?;
        }
        exponent >>= 1;
        if exponent > 0 {
            base = base.checked_mul(base).ok_or(DefiError::Overflow {
                operation: "bonding curve checked_pow square base".into(),
            })?;
        }
    }
    Ok(result)
}

fn u128_to_usize(value: u128) -> DefiResult<usize> {
    usize::try_from(value).map_err(|_| DefiError::InvalidAmount {
        reason: "bonding curve iteration size exceeds supported range".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: u8) -> Address {
        let mut bytes = [0u8; 32];
        bytes[31] = n;
        Address::from_bytes(bytes)
    }

    fn exponential_market() -> SubscriptionToken {
        SubscriptionToken::new(
            GeographicRegion::new("us-ca-bay").unwrap(),
            DataTier::Forecast,
            BondingCurve::Exponential {
                base_price: Amount::new(100),
                growth_bps: 2_000,
            },
            3_300,
            DEFAULT_ACCESS_DURATION_SECS,
            PricingGuardrails {
                max_outstanding_supply: Amount::new(8),
                max_price_multiplier_bps: 30_000,
            },
        )
        .unwrap()
    }

    #[test]
    fn exponential_curve_is_primary_mode_and_increases_with_supply() {
        let market = exponential_market();
        let p0 = market.curve.marginal_price(Amount::ZERO, &market.guardrails).unwrap();
        let p1 = market.curve.marginal_price(Amount::new(1), &market.guardrails).unwrap();
        let p2 = market.curve.marginal_price(Amount::new(2), &market.guardrails).unwrap();

        assert_eq!(p0, Amount::new(100));
        assert!(p1 > p0);
        assert!(p2 > p1);
    }

    #[test]
    fn guardrails_cap_explosive_growth_before_stack_like_amplification() {
        let guardrails = PricingGuardrails {
            max_outstanding_supply: Amount::new(100),
            max_price_multiplier_bps: 15_000,
        };
        let curve = BondingCurve::Exponential {
            base_price: Amount::new(1_000),
            growth_bps: 8_000,
        };

        let uncapped = curve
            .marginal_price(
                Amount::new(20),
                &PricingGuardrails {
                    max_outstanding_supply: guardrails.max_outstanding_supply,
                    max_price_multiplier_bps: u64::MAX / 2,
                },
            )
            .unwrap();
        let capped = curve.marginal_price(Amount::new(20), &guardrails).unwrap();

        assert!(uncapped > capped);
        assert_eq!(capped, Amount::new(1_500));
    }

    #[test]
    fn buy_routes_budget_between_reserve_and_contributor_pool() {
        let mut market = exponential_market();
        let buyer = addr(1);
        let quote = market.buy(buyer, Amount::new(400)).unwrap();

        assert_eq!(quote.tokens_minted, Amount::new(3));
        assert_eq!(quote.total_spent, Amount::new(364));
        assert_eq!(quote.reserve_added, Amount::new(119));
        assert_eq!(quote.contributor_funding, Amount::new(245));
        assert_eq!(quote.unspent_budget, Amount::new(36));
        assert_eq!(market.balance_of(&buyer), Amount::new(3));
        assert_eq!(market.supply, Amount::new(3));
        assert_eq!(market.reserve, Amount::new(119));
        assert_eq!(market.contributor_funding, Amount::new(245));
    }

    #[test]
    fn sell_returns_only_reserve_backed_portion_of_reverse_curve_value() {
        let mut market = exponential_market();
        let buyer = addr(1);
        market.buy(buyer, Amount::new(400)).unwrap();

        let quote = market.sell(buyer, Amount::new(2)).unwrap();

        assert_eq!(quote.tokens_burned, Amount::new(2));
        assert_eq!(quote.gross_curve_value, Amount::new(264));
        assert_eq!(quote.reserve_refund, Amount::new(86));
        assert_eq!(market.balance_of(&buyer), Amount::new(1));
        assert_eq!(market.supply, Amount::new(1));
        assert_eq!(market.reserve, Amount::new(33));
        assert_eq!(market.contributor_funding, Amount::new(245));
    }

    #[test]
    fn max_supply_guardrail_stops_oversubscription() {
        let mut market = exponential_market();
        let buyer = addr(2);

        let quote = market.buy(buyer, Amount::new(5_000)).unwrap();
        assert_eq!(quote.tokens_minted, Amount::new(8));
        assert_eq!(market.supply, Amount::new(8));

        let err = market.buy(buyer, Amount::new(500)).unwrap_err();
        assert!(matches!(err, DefiError::InvalidAmount { .. }));
    }

    #[test]
    fn polynomial_curve_fits_naturally_for_higher_tier_pricing() {
        let curve = BondingCurve::Polynomial {
            base_price: Amount::new(75),
            coefficient: Amount::new(10),
            exponent: 2,
        };
        let guardrails = PricingGuardrails {
            max_outstanding_supply: Amount::new(20),
            max_price_multiplier_bps: 100_000,
        };

        let p0 = curve.marginal_price(Amount::ZERO, &guardrails).unwrap();
        let p3 = curve.marginal_price(Amount::new(3), &guardrails).unwrap();
        let p5 = curve.marginal_price(Amount::new(5), &guardrails).unwrap();

        assert_eq!(p0, Amount::new(75));
        assert_eq!(p3, Amount::new(165));
        assert_eq!(p5, Amount::new(325));
    }

    #[test]
    fn full_round_trip_leaves_contributor_funding_as_non_reserve_revenue() {
        let mut market = exponential_market();
        let buyer = addr(3);
        let purchase = market.buy(buyer, Amount::new(400)).unwrap();
        let sale = market.sell(buyer, purchase.tokens_minted).unwrap();

        assert_eq!(market.supply, Amount::ZERO);
        assert_eq!(market.reserve, Amount::ZERO);
        assert_eq!(sale.reserve_refund, purchase.reserve_added);
        assert_eq!(market.contributor_funding, Amount::new(245));
    }

    #[test]
    fn token_id_is_derived_from_region_and_data_tier() {
        let market = exponential_market();
        assert_eq!(
            market.token_id.as_str(),
            "weather-subscription:us-ca-bay:forecast"
        );
    }
}
