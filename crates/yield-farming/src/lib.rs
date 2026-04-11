pub mod escrow;
pub mod farm;
pub mod rewards;
pub mod strategy;
pub mod utility_hooks;
pub mod vault;

pub use farm::{FarmManager, FarmPool};
pub use escrow::{
    EscrowSettlement, EscrowTelemetry, EscrowedReward, ForfeitReason, QualityOutcomeSignalSource,
    QualityVerificationOutcome, RewardEscrowBook, RewardEscrowPolicy,
};
pub use rewards::{RewardCalculator, VestingSchedule};
pub use strategy::{CompoundingStrategy, YieldStrategy};
pub use utility_hooks::{
    UtilityBacktestPoint, UtilityBacktestTelemetry, UtilityRewardHook, UtilityRewardHookConfig,
};
pub use vault::{
    AutoCompoundingVault, CompoundOutcome, GovernanceBoostInput, LiquidStakingInput,
    VaultGuardrails, VaultRiskSnapshot,
};
