use anchor_lang::prelude::*;

#[error_code]
pub enum StateError {
    #[msg("Active feed count exceeds maximum limit")]
    TooManyActiveFeeds,
    #[msg("Excessive liquidity provider concentration detected")]
    ExcessiveLpConcentration,
    #[msg("Price manipulation detected")]
    ManipulationDetected,
    #[msg("Caller does not have sufficient permissions for this operation")]
    InsufficientPermissions,
    #[msg("Caller is not authorized to perform this operation")]
    UnauthorizedCaller,
    #[msg("Too many active multisig members")]
    TooManyActiveMembers,
    #[msg("Asset seed does not match canonical asset ID hash")]
    InvalidAssetSeed,
    #[msg("Invalid asset ID: must be non-empty and <= 64 characters")]
    InvalidAssetId,
    #[msg("Invalid member key: cannot be default Pubkey")]
    InvalidMemberKey,
    #[msg("Duplicate member found in initial members list")]
    DuplicateMember,
    #[msg("Authority must be an initial member with admin permissions")]
    AuthorityNotAdminMember,
    #[msg("Invalid TWAP window: must be > 0 and. <= 345_600 seconds (96 hours)")]
    InvalidTWAPWindow,
    #[msg("Invalid confidence threshold: must be <= 10_000 basis points (100%)")]
    InvalidConfidenceThreshold,
    #[msg("Invalid manipulation threshold: must be > 0 and <= 10_000 basis points (100%)")]
    InvalidManipulationThreshold,
    #[msg("Invalid member count: must be > 0 and <= 16")]
    InvalidMemberCount,
    #[msg("Invalid multisig threshold: must be > 0 and <= member count")]
    InvalidMultisigThreshold,
    #[msg("Invalid emergency admin: cannot be default Pubkey")]
    InvalidEmergencyAdmin,
    #[msg("Invalid quorum threshold: must be > 0 and <= 10_000 basis points")]
    InvalidQuorumThreshold,
    #[msg("Invalid timing parameters: voting_period must be > 0, execution_delay >= 0")]
    InvalidTimingParameters,
    #[msg("Invalid proposal threshold: must be > 0")]
    InvalidProposalThreshold,
    #[msg("Too many price feeds registered")]
    TooManyFeeds,
    #[msg("Circuit breaker is currently active")]
    CircuitBreakerActive,
    #[msg("Invalid source address: cannot be default")]
    InvalidSourceAddress,
    #[msg("Unauthorized feed registration")]
    UnauthorizedFeedRegistration,
    #[msg("Invalid feed weight: must be > 0 and <= MAX_FEED_WEIGHT")]
    InvalidFeedWeight,
    #[msg("Total weight would exceed maximum allowed")]
    ExcessiveTotalWeight,
    #[msg("Duplicate feed source address")]
    DuplicateFeedSource,
    #[msg("Insufficient source liquidity")]
    InsufficientSourceLiquidity,
    #[msg("External oracle staleness threshold too high")]
    ExcessiveExternalStaleness,
    #[msg("TWAP Calculation Error: Not Enough History")]
    NotEnoughHistory,
    #[msg("Invalid Account due to owner mismatch")]
    InvalidAccount,
    #[msg("No active price feeds available")]
    NoActiveFeeds,
    #[msg("Low confidence in the fetched prices")]
    LowConfidence,
    #[msg("Mismatched price exponents in TWAP calculation")]
    MismatchedExponent,
    #[msg("Non-monotonic timestamps detected in price data")]
    NonMonotonicTimestamps,
}

#[error_code]
pub enum RaydiumObserverError {
    #[msg("Raydium CLMM Observer: Invalid account owner")]
    InvalidOwner,
    #[msg("Raydium CLMM Observer: Account too small")]
    TooSmall,
    #[msg("Raydium CLMM Observer: Uninitialized observation state")]
    Uninitialized,
    #[msg("Raydium CLMM Observer: Invalid PDA derivation")]
    BadPda,
    #[msg("Raydium CLMM Observer: pool.observation_key mismatch with oracle")]
    PoolMismatch,
    #[msg("Raydium CLMM Observer: Invalid window")]
    InvalidWindow,
    #[msg("Raydium CLMM Observer: Invalid observation index")]
    InvalidIndex,
    #[msg("Raydium CLMM Observer: Insufficient time elapsed")]
    InsufficientTime,
    #[msg("Raydium CLMM Observer: Tick out of bounds")]
    TickOutOfBounds,
    #[msg("Raydium CLMM Observer: Math Error")]
    MathError,
    #[msg("Raydium CLMM Observer: Excessive tick deviation")]
    ExcessiveDeviation,
    #[msg("Update Price Instruction: Invalid Observation PDA")]
    InvalidObservationPda,
    #[msg("Update Price Instruction: Invalid TWAP price fetched")]
    InvalidPrice,
}