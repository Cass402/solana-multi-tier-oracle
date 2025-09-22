/// Efficient sqrt price to tick conversion algorithms using bit manipulation and fixed-point arithmetic.
/// 
/// # Mathematical Foundation
/// 
/// This module implements the core mathematical relationship between ticks and prices in CLMM systems:
/// price = 1.0001^tick, where tick represents discrete price levels and price is the ratio between tokens.
/// 
/// # Performance Strategy
/// 
/// Uses bit decomposition and precomputed constants to achieve O(log n) complexity for price calculations
/// without expensive floating-point operations or iterative approximations. This approach is critical
/// for oracle operations where price conversions occur frequently within compute unit constraints.
/// 
/// # Precision Requirements
/// 
/// All calculations use Q64.64 fixed-point format to maintain sufficient precision for financial
/// applications while ensuring deterministic results across different hardware platforms.

use crate::error::RaydiumObserverError;
use anchor_lang::prelude::*;
use crate::components::raydium_clmm_observer::raydium_constants::{MAX_TICK, MIN_TICK,MAX_SQRT_PRICE_X64, MIN_SQRT_PRICE_X64};
use ethnum::U256;

/// Precomputed powers of 10 for efficient decimal scaling operations.
/// 
/// # Lookup Table Strategy
/// 
/// Avoids expensive exponentiation during decimal conversions by precomputing commonly
/// needed powers of 10. The range covers typical token decimal differences (0-18),
/// enabling efficient price scaling between tokens with different precision requirements.
/// 
/// # Memory vs Computation Trade-off
/// 
/// This lookup table consumes ~304 bytes of constant memory but eliminates repeated
/// exponentiation calculations during price conversions, providing significant
/// performance benefits for high-frequency oracle operations.
const POW10_LOOKUP: [u128; 19] = [
    1, 10, 100, 1_000, 10_000, 100_000, 1_000_000, 10_000_000, 100_000_000,
    1_000_000_000, 10_000_000_000, 100_000_000_000, 1_000_000_000_000,
    10_000_000_000_000, 100_000_000_000_000, 1_000_000_000_000_000,
    10_000_000_000_000_000, 100_000_000_000_000_000, 1_000_000_000_000_000_000,
];

/// Multiply two Q64.64 fixed-point numbers with overflow protection.
/// 
/// # Fixed-Point Arithmetic Strategy
/// 
/// Performs multiplication in U256 intermediate space to prevent overflow during
/// the intermediate product calculation, then shifts right by 64 bits to maintain
/// Q64.64 format. This approach ensures mathematical correctness while providing
/// overflow detection for safety.
/// 
/// # Overflow Safety Design
/// 
/// Uses U256 arithmetic to handle intermediate overflow conditions that would
/// cause silent wraparound in u128 arithmetic. This is critical for financial
/// calculations where overflow could lead to incorrect price computations.
#[inline(always)]
fn multiply_q64(a: u128, b: u128) -> Result<u128> {
    let product = U256::from(a) * U256::from(b);
    let shifted: U256 = product >> 64;
    
    // Check for overflow beyond u128 representation
    if shifted > U256::from(u128::MAX) {
        return Err(RaydiumObserverError::MathError.into());
    }
    
    Ok(shifted.as_u128())
}

/// Calculate sqrt price ratio from tick using efficient bit decomposition algorithm.
/// 
/// # Mathematical Relationship
/// 
/// Implements the formula: sqrt_price = sqrt(1.0001^tick) = 1.0001^(tick/2)
/// where 1.0001 represents the base price ratio between adjacent ticks in CLMM systems.
/// 
/// # Bit Decomposition Algorithm
/// 
/// Uses binary representation of the tick value to compose the result through
/// multiplication of precomputed factors. This approach achieves O(log n) complexity
/// by leveraging the fact that any integer can be expressed as a sum of powers of 2.
/// 
/// # Algorithmic Efficiency
/// 
/// For tick = 100 (binary: 1100100), the algorithm multiplies factors for bits
/// 2, 5, and 6, avoiding 16+ sequential multiplications. Each bit position
/// corresponds to a precomputed factor representing 1.0001^(2^bit_position / 2).
/// 
/// # Precision Strategy
/// 
/// All factors are precomputed with maximum precision and stored as Q64.64 constants.
/// This eliminates floating-point operations while maintaining sufficient precision
/// for financial calculations across the entire valid tick range.
#[inline(always)]
pub fn get_sqrt_ratio_at_tick(tick: i32) -> Result<u128> {
    // Validate tick is within bounds to prevent undefined mathematical behavior
    require!(tick >= -MAX_TICK && tick <= MAX_TICK, RaydiumObserverError::TickOutOfBounds);

    // Handle extreme boundary cases with precomputed constants to avoid precision loss
    if tick == MIN_TICK {
        return Ok(MIN_SQRT_PRICE_X64);
    }

    if tick == MAX_TICK {
        return Ok(MAX_SQRT_PRICE_X64);
    }

    let abs_tick = tick.abs() as u32;

    // Precomputed sqrt(1.0001^(2^n / 2)) factors for bit decomposition algorithm.
    // Each constant represents 1.0001^(2^bit_position / 2) in Q64.64 fixed-point format.
    // These values are mathematically derived and critical for algorithmic correctness.
    
    const FN1: u128    = 0xFFFcb933bd6fad37;    // 1.0001^(1/2)   = sqrt(1.0001^1)
    const FN2: u128    = 0xFFF97272373d413c;    // 1.0001^(2/2)   = sqrt(1.0001^2)
    const FN4: u128    = 0xFFF2e50f5f656932;    // 1.0001^(4/2)   = sqrt(1.0001^4)
    const FN8: u128    = 0xFFE5caca7e10e4e6;    // 1.0001^(8/2)   = sqrt(1.0001^8)
    const FN16: u128   = 0xFFCB9843d60f6159;    // 1.0001^(16/2)  = sqrt(1.0001^16)
    const FN32: u128   = 0xFF973b41fa98c081;    // 1.0001^(32/2)  = sqrt(1.0001^32)
    const FN64: u128   = 0xFF2ea16466c96a39;    // 1.0001^(64/2)  = sqrt(1.0001^64)
    const FN128: u128  = 0xFE5dee046a99a2a8;    // 1.0001^(128/2) = sqrt(1.0001^128)
    const FN256: u128  = 0xFCbe86c7900a88ae;    // 1.0001^(256/2) = sqrt(1.0001^256)
    const FN512: u128  = 0xF987a7253ac41317;    // 1.0001^(512/2) = sqrt(1.0001^512)
    const FN1024: u128 = 0xF3392b0822b70005;    // 1.0001^(1024/2) = sqrt(1.0001^1024)
    const FN2048: u128 = 0xE7159475a2c29b74;    // 1.0001^(2048/2) = sqrt(1.0001^2048)
    const FN4096: u128 = 0xD097f3bdfd2022b8;    // 1.0001^(4096/2) = sqrt(1.0001^4096)
    const FN8192: u128 = 0xA9f746462d870fdf;    // 1.0001^(8192/2) = sqrt(1.0001^8192)
    const FN16384: u128= 0x70d869a156d2a1b8;    // 1.0001^(16384/2) = sqrt(1.0001^16384)
    const FN32768: u128= 0x31be135f97d08fd9;    // 1.0001^(32768/2) = sqrt(1.0001^32768)
    const FN65536: u128= 0x9aa508b5b7a84e1c;    // 1.0001^(65536/2) = sqrt(1.0001^65536)
    const FN131072: u128= 0x5d6af8dedb81196d;   // 1.0001^(131072/2) = sqrt(1.0001^131072)
    const FN262144: u128= 0x2216e584f5fa1ea9;   // 1.0001^(262144/2) = sqrt(1.0001^262144)

    // Initialize ratio based on least significant bit (odd/even tick handling)
    // Odd ticks require multiplication by FN1, even ticks start with 1.0 (1 << 64 in Q64.64)
    let mut ratio = if (abs_tick & 1) != 0 { FN1 } else { 1u128 << 64 };

    // Binary decomposition: multiply by factor for each set bit in tick value
    // This transforms O(n) sequential multiplications into O(log n) bit operations
    if (abs_tick & (1 << 1)) != 0 {
        ratio = multiply_q64(ratio, FN2)?;
    }
    if (abs_tick & (1 << 2)) != 0 {
        ratio = multiply_q64(ratio, FN4)?;
    }
    if (abs_tick & (1 << 3)) != 0 {
        ratio = multiply_q64(ratio, FN8)?;
    }
    if (abs_tick & (1 << 4)) != 0 {
        ratio = multiply_q64(ratio, FN16)?;
    }
    if (abs_tick & (1 << 5)) != 0 {
        ratio = multiply_q64(ratio, FN32)?;
    }
    if (abs_tick & (1 << 6)) != 0 {
        ratio = multiply_q64(ratio, FN64)?;
    }
    if (abs_tick & (1 << 7)) != 0 {
        ratio = multiply_q64(ratio, FN128)?;
    }
    if (abs_tick & (1 << 8)) != 0 {
        ratio = multiply_q64(ratio, FN256)?;
    }
    if (abs_tick & (1 << 9)) != 0 {
        ratio = multiply_q64(ratio, FN512)?;
    }
    if (abs_tick & (1 << 10)) != 0 {
        ratio = multiply_q64(ratio, FN1024)?;
    }
    if (abs_tick & (1 << 11)) != 0 {
        ratio = multiply_q64(ratio, FN2048)?;
    }
    if (abs_tick & (1 << 12)) != 0 {
        ratio = multiply_q64(ratio, FN4096)?;
    }
    if (abs_tick & (1 << 13)) != 0 {
        ratio = multiply_q64(ratio, FN8192)?;
    }
    if (abs_tick & (1 << 14)) != 0 {
        ratio = multiply_q64(ratio, FN16384)?;
    }
    if (abs_tick & (1 << 15)) != 0 {
        ratio = multiply_q64(ratio, FN32768)?;
    }
    if (abs_tick & (1 << 16)) != 0 {
        ratio = multiply_q64(ratio, FN65536)?;
    }
    if (abs_tick & (1 << 17)) != 0 {
        ratio = multiply_q64(ratio, FN131072)?;
    }
    if (abs_tick & (1 << 18)) != 0 {
        ratio = multiply_q64(ratio, FN262144)?;
    }

    // Handle negative ticks by taking reciprocal: 1.0001^(-tick) = 1 / 1.0001^tick
    // Uses U256 division to maintain precision during reciprocal calculation
    if tick > 0 {
        ratio = (U256::from(u128::MAX) / U256::from(ratio)).as_u128();
    }

    // Validate final result is within representable sqrt price bounds
    require!(ratio >= MIN_SQRT_PRICE_X64 && ratio <= MAX_SQRT_PRICE_X64, RaydiumObserverError::MathError);

    Ok(ratio)
}

/// Convert sqrt price to human-readable price with proper decimal scaling.
/// 
/// # Price Calculation Mathematics
/// 
/// Computes the actual token ratio: price = (sqrt_price)² = token1_amount / token0_amount
/// The sqrt representation is used in CLMM for efficient math operations, but end users
/// need the actual price ratio for meaningful interpretation.
/// 
/// # Decimal Scaling Strategy
/// 
/// Adjusts for different token decimal precision to produce human-readable prices.
/// Example: USDC (6 decimals) / ETH (18 decimals) requires 10^(6-18) = 10^(-12) scaling
/// to display price correctly as "USDC per ETH" rather than raw integer ratios.
/// 
/// # Rounding Strategy for Division
/// 
/// When scaling down (negative decimal difference), adds half the divisor before division
/// to implement banker's rounding. This prevents systematic bias that could accumulate
/// in repeated calculations and provides more accurate price representations.
/// 
/// # Performance Optimization
/// 
/// Uses precomputed powers of 10 lookup table to avoid expensive exponentiation during
/// decimal conversions. Handles the most common decimal differences (±18) efficiently.
#[inline(always)]
pub fn ui_price_from_sqrt_q64(sqrt_price_x64: u128, decimal_0: u8, decimal_1: u8) -> Result <u128> {
    // Calculate actual price by squaring sqrt price: price = (sqrt_price)²
    // This converts from sqrt representation back to actual token ratio
    let price_x64 = multiply_q64(sqrt_price_x64, sqrt_price_x64)?;

    // Convert from Q64.64 fixed-point to integer by removing fractional bits
    let price = price_x64 >> 64;

    // Calculate decimal adjustment needed for human-readable price display
    // Positive: token0 has more decimals, need to multiply to scale up
    // Negative: token1 has more decimals, need to divide to scale down
    let decimal_difference = decimal_0 as i8 - decimal_1 as i8;

    let scaled = match decimal_difference {
        // No decimal adjustment needed - tokens have same precision
        0 => price,
        
        // Scale up: token0 has more decimals than token1
        // Multiply by 10^difference to adjust for decimal disparity
        1..=18 => price.saturating_mul(POW10_LOOKUP[decimal_difference as usize]),
        
        // Scale down: token1 has more decimals than token0
        // Divide by 10^|difference| with rounding for accuracy
        -18..=-1 => {
            let divisor = POW10_LOOKUP[(-decimal_difference) as usize];
            // Add half divisor before division for banker's rounding
            // This prevents systematic bias in repeated calculations
            (price + (divisor >> 1)) / divisor
        },
        
        // Decimal difference exceeds lookup table range - unsupported
        _ => return Err(RaydiumObserverError::MathError.into()),
    };

    Ok(scaled)
}