use crate::error::{RevenueError, RevenueResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::types::Address;

/// Minimum quality score (bps) for a contributor to be eligible for distribution.
pub const MIN_QUALITY_THRESHOLD_BPS: u16 = 5_000; // 50%

/// Profile for a weather data contributor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContributorProfile {
    pub address: Address,
    /// Data quality score in basis points (0–10_000). From physics validation pass rate.
    pub quality_score_bps: u16,
    /// Station uptime in basis points (0–10_000).
    pub uptime_bps: u16,
    /// Geographic scarcity bonus in basis points (0–10_000). Higher in data deserts.
    pub geographic_score_bps: u16,
    /// Data volume relative weight in basis points (0–10_000).
    pub volume_score_bps: u16,
    /// Whether the contributor is currently active.
    pub active: bool,
}

impl ContributorProfile {
    pub fn new(address: Address) -> Self {
        Self {
            address,
            quality_score_bps: 0,
            uptime_bps: 0,
            geographic_score_bps: 0,
            volume_score_bps: 0,
            active: true,
        }
    }

    /// Weighted composite score used for distribution allocation.
    ///
    /// Formula: quality(40%) + uptime(30%) + geographic(20%) + volume(10%)
    pub fn composite_score(&self) -> u64 {
        let q = self.quality_score_bps as u64 * 40;
        let u = self.uptime_bps as u64 * 30;
        let g = self.geographic_score_bps as u64 * 20;
        let v = self.volume_score_bps as u64 * 10;
        (q + u + g + v) / 100
    }

    pub fn is_eligible(&self) -> bool {
        self.active && self.quality_score_bps >= MIN_QUALITY_THRESHOLD_BPS
    }
}

/// Registry of all weather data contributors.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DataContributorRegistry {
    contributors: HashMap<[u8; 32], ContributorProfile>,
}

impl DataContributorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, profile: ContributorProfile) {
        self.contributors.insert(profile.address.0, profile);
    }

    pub fn update(&mut self, address: Address, profile: ContributorProfile) -> RevenueResult<()> {
        self.contributors
            .get_mut(&address.0)
            .ok_or(RevenueError::ContributorNotFound(address))
            .map(|p| *p = profile)
    }

    pub fn get(&self, address: &Address) -> Option<&ContributorProfile> {
        self.contributors.get(&address.0)
    }

    pub fn eligible_contributors(&self) -> Vec<&ContributorProfile> {
        self.contributors
            .values()
            .filter(|p| p.is_eligible())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_address(byte: u8) -> Address {
        let mut bytes = [0u8; 32];
        bytes[31] = byte;
        Address::from_bytes(bytes)
    }

    fn make_profile(
        address: Address,
        quality: u16,
        uptime: u16,
        geo: u16,
        volume: u16,
        active: bool,
    ) -> ContributorProfile {
        ContributorProfile {
            address,
            quality_score_bps: quality,
            uptime_bps: uptime,
            geographic_score_bps: geo,
            volume_score_bps: volume,
            active,
        }
    }

    #[test]
    fn test_composite_score_formula() {
        // quality=10000, uptime=10000, geo=10000, volume=10000 → (40+30+20+10)*100/100 = 10000
        let profile = make_profile(make_address(1), 10_000, 10_000, 10_000, 10_000, true);
        assert_eq!(profile.composite_score(), 10_000);
    }

    #[test]
    fn test_composite_score_zero() {
        let profile = make_profile(make_address(1), 0, 0, 0, 0, true);
        assert_eq!(profile.composite_score(), 0);
    }

    #[test]
    fn test_composite_score_weighted() {
        // quality=8000 (40%), uptime=6000 (30%), geo=4000 (20%), volume=2000 (10%)
        // (8000*40 + 6000*30 + 4000*20 + 2000*10) / 100
        // = (320000 + 180000 + 80000 + 20000) / 100
        // = 600000 / 100 = 6000
        let profile = make_profile(make_address(1), 8_000, 6_000, 4_000, 2_000, true);
        assert_eq!(profile.composite_score(), 6_000);
    }

    #[test]
    fn test_is_eligible_active_above_threshold() {
        let profile = make_profile(make_address(1), MIN_QUALITY_THRESHOLD_BPS, 0, 0, 0, true);
        assert!(profile.is_eligible());
    }

    #[test]
    fn test_is_eligible_inactive() {
        let profile = make_profile(make_address(1), 10_000, 10_000, 10_000, 10_000, false);
        assert!(!profile.is_eligible());
    }

    #[test]
    fn test_is_eligible_below_quality_threshold() {
        let profile = make_profile(
            make_address(1),
            MIN_QUALITY_THRESHOLD_BPS - 1,
            10_000,
            10_000,
            10_000,
            true,
        );
        assert!(!profile.is_eligible());
    }

    #[test]
    fn test_register_and_get() {
        let mut registry = DataContributorRegistry::new();
        let addr = make_address(5);
        let profile = make_profile(addr, 8_000, 9_000, 7_000, 5_000, true);
        registry.register(profile.clone());
        let retrieved = registry.get(&addr).unwrap();
        assert_eq!(retrieved.address, addr);
        assert_eq!(retrieved.quality_score_bps, 8_000);
    }

    #[test]
    fn test_eligible_contributors_filters_correctly() {
        let mut registry = DataContributorRegistry::new();
        // Eligible
        registry.register(make_profile(
            make_address(1),
            6_000,
            8_000,
            5_000,
            3_000,
            true,
        ));
        // Ineligible — quality too low
        registry.register(make_profile(
            make_address(2),
            4_000,
            10_000,
            10_000,
            10_000,
            true,
        ));
        // Ineligible — inactive
        registry.register(make_profile(
            make_address(3),
            9_000,
            9_000,
            9_000,
            9_000,
            false,
        ));

        let eligible = registry.eligible_contributors();
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0].address, make_address(1));
    }

    #[test]
    fn test_update_existing_contributor() {
        let mut registry = DataContributorRegistry::new();
        let addr = make_address(10);
        registry.register(make_profile(addr, 5_000, 5_000, 5_000, 5_000, true));
        let updated = make_profile(addr, 9_000, 9_000, 9_000, 9_000, true);
        registry.update(addr, updated).unwrap();
        assert_eq!(registry.get(&addr).unwrap().quality_score_bps, 9_000);
    }

    #[test]
    fn test_update_nonexistent_returns_error() {
        let mut registry = DataContributorRegistry::new();
        let addr = make_address(99);
        let profile = make_profile(addr, 9_000, 9_000, 9_000, 9_000, true);
        let result = registry.update(addr, profile);
        assert!(matches!(result, Err(RevenueError::ContributorNotFound(_))));
    }
}
