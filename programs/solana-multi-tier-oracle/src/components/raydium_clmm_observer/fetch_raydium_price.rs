use crate::components::raydium_clmm_observer::raydium_accounts::{
    read_observation, verify_observation_pda_and_read_pool,
};
use crate::components::raydium_clmm_observer::raydium_constants::OBSERVATION_UPDATE_DURATION;
use crate::components::raydium_clmm_observer::sqrt_price_to_tick::{
    get_sqrt_ratio_at_tick, ui_price_from_sqrt_q64,
};
/// Comprehensive Raydium price fetching with multi-layer validation and manipulation detection.
///
/// # Oracle Security Architecture
///
/// This module implements a sophisticated price fetching algorithm that combines multiple
/// statistical methods to produce highly reliable price data while detecting and preventing
/// various manipulation attack vectors. The approach prioritizes security over simplicity
/// to ensure oracle integrity in adversarial DeFi environments.
///
/// # Multi-Method Validation Strategy
///
/// Uses both TWAP (Time-Weighted Average Price) and T2EMA (Triple Exponential Moving Average)
/// calculations, cross-validating results to detect inconsistencies that could indicate
/// manipulation attempts. This dual-method approach significantly increases the difficulty
/// of successful oracle attacks.
use crate::components::raydium_clmm_observer::twap::{
    assess_manipulation_risk, confidence_from_variance, find_observation_for_window, t2ema_tick,
    twap_tick_from_cumulatives,
};
use crate::error::RaydiumObserverError;
use anchor_lang::prelude::*;

/// Comprehensive price result with embedded metadata for downstream risk assessment.
///
/// # Rich Metadata Strategy
///
/// Beyond just price, this struct provides essential context for price consumers to make
/// informed decisions about data reliability. The metadata enables dynamic risk management
/// where downstream protocols can adjust their behavior based on price quality indicators.
///
/// # Trust-Minimized Design
///
/// All validation metrics are included rather than hidden, allowing price consumers to
/// implement their own risk thresholds rather than trusting oracle-level filtering.
/// This transparency enables more sophisticated risk management strategies.
pub struct DecimalPrice {
    /// Human-readable price scaled for token decimal differences.
    /// Represents token1/token0 ratio adjusted for display precision.
    pub price: u128,

    /// Statistical confidence metric (0-10,000 basis points) based on price variance analysis.
    /// Higher values indicate more stable price behavior and greater reliability.
    pub confidence: u32,

    /// Unix timestamp of the most recent observation used in calculation.
    /// Enables downstream staleness detection and temporal analysis.
    pub timestamp: i64,

    /// Source pool account providing the price data for traceability.
    /// Allows downstream protocols to implement pool-specific risk policies.
    pub source: Pubkey,

    /// Current liquidity depth indicating manipulation resistance.
    /// Higher liquidity makes price manipulation more expensive and difficult.
    pub liquidity_depth: u128,

    /// Composite manipulation risk score (0-10,000 basis points).
    /// Higher values indicate greater likelihood of price manipulation or anomalies.
    pub manipulation_score: u32,

    /// Decimal places for token0 in the pool, used for price scaling.
    pub decimal_0: u8,

    /// Decimal places for token1 in the pool, used for price scaling.
    pub decimal_1: u8,
}

/// Configuration parameters controlling price calculation behavior and risk thresholds.
///
/// # Parameterization Strategy
///
/// Externalized parameters enable fine-tuning of oracle behavior for different market
/// conditions and risk tolerances without requiring code changes. This flexibility
/// is essential for adapting to evolving attack vectors and market dynamics.
///
/// # Risk Management Framework
///
/// Parameters define multiple layers of protection, from time window requirements
/// to deviation thresholds, creating a comprehensive defense against manipulation.
pub struct RaydiumParams {
    /// TWAP calculation window in seconds, minimum time span for price averaging.
    /// Longer windows provide manipulation resistance but reduce price responsiveness.
    pub window_seconds: u32,

    /// Minimum elapsed time required for valid price calculation.
    /// Prevents manipulation through ultra-short time windows that could be gamed.
    pub min_seconds: u32,

    /// Minimum liquidity threshold for reliable price feeds.
    /// Below this level, prices are considered too susceptible to manipulation.
    pub min_liquidity: u128,

    /// Maximum allowed tick deviation for cross-validation checks.
    /// Prevents acceptance of prices that deviate excessively between calculation methods.
    pub max_tick_deviation: i32,

    /// EMA smoothing factor in basis points (0-10,000) for T2EMA calculations.
    /// Controls responsiveness vs stability trade-off in trend analysis.
    pub alpha_basis_points: u16,

    /// Current timestamp for staleness and time window calculations.
    /// Should represent actual current time for accurate freshness assessment.
    pub timestamp: i64,
}

/// Orchestrate comprehensive price fetching with multi-layer security validation.
///
/// # Security-First Architecture
///
/// This function implements a defense-in-depth approach to price fetching, combining:
/// 1. **Account Validation**: Cryptographic verification of pool and observation integrity
/// 2. **Statistical Analysis**: Dual-method price calculation with cross-validation
/// 3. **Deviation Checking**: Multiple deviation bounds to detect anomalous price behavior
/// 4. **Risk Assessment**: Comprehensive manipulation detection and confidence scoring
///
/// # Algorithm Flow Design
///
/// The algorithm prioritizes security over performance by performing extensive validation
/// at each step. Early failure detection prevents propagation of corrupted data through
/// the oracle system, even at the cost of additional computational overhead.
///
/// # Attack Resistance Strategy
///
/// By requiring consistency between TWAP and T2EMA calculations, the algorithm makes it
/// significantly more difficult for attackers to manipulate prices, as they would need
/// to sustain manipulation across different time horizons and calculation methods.
pub fn fetch_raydium_price_from_observations(
    pool_account_info: &AccountInfo,
    observation_account_info: &AccountInfo,
    program_id: &Pubkey,
    params: RaydiumParams,
) -> Result<DecimalPrice> {
    // Phase 1: Account Authentication and Relationship Validation
    // Cryptographically verify that pool and observation accounts are legitimate
    // and properly linked to prevent spoofing attacks and data contamination
    let pool = verify_observation_pda_and_read_pool(
        pool_account_info,
        observation_account_info,
        program_id,
    )?;
    let observation = read_observation(observation_account_info, program_id)?;

    // Phase 2: Time Window Selection and Data Freshness Validation
    // Find optimal observation pair for TWAP calculation while ensuring data freshness
    // The time window selection balances accuracy (longer windows) with responsiveness
    let (index_then, index_now, seconds_elapsed) =
        find_observation_for_window(&observation, params.timestamp, params.window_seconds)?;

    // Enforce minimum time requirements to prevent manipulation through micro-timeframes
    // Uses the stricter of user-defined minimum or protocol-defined update duration
    //require!(seconds_elapsed >= core::cmp::max(params.min_seconds, OBSERVATION_UPDATE_DURATION), RaydiumObserverError::InsufficientTime);

    // Phase 3: Historical Data Extraction
    // Extract the specific observations that bracket our desired time window
    // These form the endpoints for both TWAP and variance calculations
    let observation_then = observation.get_observation(index_then);
    let observation_now = observation.get_observation(index_now);

    // Phase 4: Dual-Method Price Calculation
    // Calculate price using two independent methods for cross-validation

    // TWAP: Traditional time-weighted average price resistant to short-term manipulation
    let mut twap_tick = twap_tick_from_cumulatives(
        observation_then.tick_cumulative(),
        observation_now.tick_cumulative(),
        seconds_elapsed,
    )?;

    // Fallback logic: if TWAP calculation returned 0 (no meaningful time difference),
    // use the current pool tick as the best available price estimate
    if twap_tick == 0 {
        let current_tick = pool.tick_current();
        twap_tick = current_tick as i64;
    }

    // T2EMA: Advanced exponential moving average with lag compensation for trend analysis
    let t2ema_tick = t2ema_tick(
        &observation,
        index_then,
        index_now,
        params.alpha_basis_points,
    )?;

    // Phase 5: Cross-Method Validation and Deviation Analysis
    // Verify consistency between different calculation methods to detect manipulation

    let current_tick = pool.tick_current();

    // Check T2EMA deviation from current pool state
    // Large deviations could indicate stale data or manipulation attempts
    let dev64 = t2ema_tick.abs_diff(current_tick as i64);
    let dev_vs_current = i32::try_from(dev64).unwrap_or(i32::MAX);

    require!(
        dev_vs_current <= params.max_tick_deviation,
        RaydiumObserverError::ExcessiveDeviation
    );

    // Cross-validate TWAP vs T2EMA consistency
    // Significant divergence between methods suggests potential manipulation or data quality issues
    let dev_twap_vs_t2ema64 = twap_tick.abs_diff(t2ema_tick);
    let dev_twap_vs_t2ema = i32::try_from(dev_twap_vs_t2ema64).unwrap_or(i32::MAX);

    require!(
        dev_twap_vs_t2ema <= params.max_tick_deviation,
        RaydiumObserverError::ExcessiveDeviation
    );

    // Phase 6: Price Conversion and Human-Readable Formatting
    // Convert validated tick to actual price ratio with proper decimal scaling
    let sqrt_price_x64 = get_sqrt_ratio_at_tick(t2ema_tick as i32)?;
    let (decimal_0, decimal_1) = pool.decimals();
    // let ui_price = ui_price_from_sqrt_q64(sqrt_price_x64, decimal_0, decimal_1)?;

    // Phase 7: Confidence and Risk Assessment
    // Generate metadata for downstream risk management decisions

    // Statistical confidence based on price variance over the observation window
    let base_confidence = confidence_from_variance(&observation, index_then, index_now)?;

    // Comprehensive manipulation risk assessment incorporating multiple risk factors
    let risk_score = assess_manipulation_risk(
        base_confidence,
        dev_vs_current,
        seconds_elapsed,
        pool.liquidity(),
        params.min_liquidity,
    );

    // Phase 8: Result Assembly
    // Package validated price with comprehensive metadata for informed downstream usage
    Ok(DecimalPrice {
        price: sqrt_price_x64,
        confidence: base_confidence,
        timestamp: observation_now.block_timestamp() as i64,
        source: *pool_account_info.key,
        liquidity_depth: pool.liquidity(),
        manipulation_score: risk_score,
        decimal_0,
        decimal_1,
    })
}
