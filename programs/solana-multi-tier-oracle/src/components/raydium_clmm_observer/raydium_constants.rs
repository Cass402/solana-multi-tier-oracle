use anchor_lang::prelude::*;

/// Raydium CLMM program identifiers for cross-program invocations and account validation.
/// 
/// # Network Separation Strategy
/// 
/// Separate constants for mainnet and devnet enable environment-specific deployments
/// while preventing accidental cross-network interactions that could cause runtime failures.
/// The oracle system validates account ownership against these program IDs to ensure
/// it only reads authentic Raydium pool data and prevents spoofing attacks.

/// Production Raydium CLMM program deployment on Solana mainnet.
/// Used for account ownership validation in production oracle operations.
pub const RAYDIUM_CLMM_PROGRAM_ID_MAINNET: Pubkey = pubkey!("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK");

/// Development Raydium CLMM program deployment for testing and integration.
/// Enables oracle development and testing without affecting production pools or incurring mainnet costs.
pub const RAYDIUM_CLMM_PROGRAM_ID_DEVNET: Pubkey = pubkey!("DRayAUgENGQBKVaX8owNhgzkEDyoHTGVEGHVJT1E9pfH");

/// Configuration constants for Raydium observation system integration.
/// 
/// # TWAP Implementation Parameters
/// 
/// These constants define the observation system's behavior for time-weighted average
/// price calculations. The values are chosen to balance price accuracy with storage
/// efficiency and update frequency requirements for DeFi oracle applications.

/// PDA seed for deriving observation account addresses.
/// Ensures deterministic account generation while preventing address collisions with other account types.
pub const OBSERVATION_SEED: &[u8] = b"observation";

/// Fixed size of the circular observation buffer for TWAP calculations.
/// 
/// # Buffer Size Rationale
/// 
/// 100 observations provides sufficient historical depth for meaningful TWAP calculations
/// while maintaining reasonable account storage costs. This size supports:
/// - ~25 minutes of history at 15-second update intervals
/// - Adequate samples for statistical price analysis
/// - Fixed account size for predictable rent calculations
/// - Memory efficiency for frequent zero-copy access operations
pub const OBSERVATION_NUM: usize = 100;

/// Minimum interval between observation updates in seconds.
/// 
/// # Update Frequency Design
/// 
/// 15-second intervals balance several competing requirements:
/// - **Price Responsiveness**: Frequent enough to capture meaningful price movements
/// - **Computational Efficiency**: Reduces update transaction frequency and associated costs
/// - **Storage Optimization**: Prevents observation buffer churn from high-frequency updates
/// - **Network Congestion**: Avoids contributing to network spam during high-activity periods
/// 
/// This interval aligns with typical DeFi price update patterns while ensuring TWAP accuracy.
pub const OBSERVATION_UPDATE_DURATION: u32 = 15;

/// Raydium CLMM tick range boundaries defining valid price ranges.
/// 
/// # Tick System Design
/// 
/// Raydium uses a tick-based pricing system where each tick represents a discrete price level.
/// These bounds prevent overflow in tick arithmetic and ensure all pool operations remain
/// within mathematically valid ranges for fixed-point price calculations.

/// Minimum valid tick value representing the lowest possible price ratio.
/// Corresponds to extremely low token1/token0 ratios near mathematical limits.
pub const MIN_TICK: i32 = -443_636;

/// Maximum valid tick value representing the highest possible price ratio.
/// Corresponds to extremely high token1/token0 ratios near mathematical limits.
pub const MAX_TICK: i32 = 443_636;

/// Raydium CLMM sqrt price bounds in Q64.64 fixed-point format.
/// 
/// # Fixed-Point Precision Strategy
/// 
/// Using Q64.64 format (64 integer bits + 64 fractional bits) provides sufficient precision
/// for financial calculations while avoiding floating-point precision issues that could
/// accumulate errors in price computations. These bounds ensure all sqrt price values
/// remain within the representable range of the fixed-point format.

/// Minimum sqrt price value in Q64.64 format.
/// Represents the lower bound of expressible price ratios to prevent underflow in price calculations.
pub const MIN_SQRT_PRICE_X64: u128 = 4_295_048_016u128;

/// Maximum sqrt price value in Q64.64 format.
/// Represents the upper bound of expressible price ratios to prevent overflow in price calculations.
pub const MAX_SQRT_PRICE_X64: u128 = 79_226_673_521_066_979_257_578_248_091u128;
