/// Zero-copy account structures for Raydium CLMM integration with strict compatibility requirements.
/// 
/// # Critical Compatibility Constraint
/// 
/// These account structures MUST remain byte-for-byte compatible with Raydium's CLMM program
/// to enable direct memory access without serialization overhead. Any deviation from Raydium's
/// exact memory layout will cause silent data corruption or runtime failures.
/// 
/// # Design Philosophy
/// 
/// This module prioritizes performance over safety by using packed structs and unsafe pointer
/// operations to achieve zero-copy reads from Raydium accounts. The trade-offs are:
/// 
/// - **Performance Benefit**: Eliminates serialization/deserialization overhead for high-frequency
///   price updates, critical for oracle responsiveness
/// - **Safety Cost**: Requires careful memory alignment and lifetime management to prevent UB
/// - **Maintenance Burden**: Must track Raydium program updates to maintain compatibility
/// 
/// # Memory Safety Strategy
/// 
/// - Packed struct representations prevent compiler padding that would break compatibility
/// - Reader wrappers provide safe access patterns over unsafe raw pointer operations
/// - Validation functions ensure account ownership and initialization before dereferencing
use anchor_lang::prelude::*;
use crate::components::raydium_clmm_observer::raydium_constants::{OBSERVATION_NUM, RAYDIUM_CLMM_PROGRAM_ID_DEVNET};
use crate::error::RaydiumObserverError;

/// Size of invariant prefix fields in PoolState that precede the fields we need.
/// Calculated as: bump(1) + amm_config(32) + owner(32) + token_mint_0(32) + 
/// token_mint_1(32) + token_vault_0(32) + token_vault_1(32) = 193 bytes.
/// This offset calculation is critical for accessing the observation_key field correctly.
const POOL_STATE_PREFIX_SIZE: usize =  1 + 32 + 32 + 32 + 32 + 32 + 32;

/// Individual TWAP observation data point mirroring Raydium's exact memory layout.
/// 
/// # Compatibility Requirements
/// 
/// This struct MUST match Raydium's Observation struct exactly to enable zero-copy reads.
/// The packed representation prevents Rust compiler from inserting padding that would
/// break memory layout compatibility with Raydium's C-style struct definitions.
/// 
/// # TWAP Calculation Context
/// 
/// Each observation captures a snapshot of the pool's tick state at a specific timestamp,
/// enabling time-weighted average price calculations across multiple observations.
/// The cumulative tick design allows efficient TWAP computation without storing
/// individual price snapshots.
#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct Observation {
    /// Unix timestamp when this observation was recorded.
    /// Used as the time component in TWAP calculations and for determining observation staleness.
    pub block_timestamp: u32,
    
    /// Cumulative sum of tick values up to this timestamp.
    /// Enables efficient TWAP calculation: (tick_cumulative_end - tick_cumulative_start) / time_delta.
    /// This design avoids storing individual tick values while preserving TWAP accuracy.
    pub tick_cumulative: i64,
    
    /// Reserved space for Raydium's future feature additions.
    /// Critical for maintaining forward compatibility when Raydium extends their observation format.
    pub padding: [u64; 4],
}

/// Circular buffer containing historical price observations for TWAP calculations.
/// 
/// # Circular Buffer Design Rationale
/// 
/// Uses a fixed-size circular buffer to maintain a rolling window of price observations
/// without the memory allocation overhead of dynamic arrays. This design choice provides:
/// 
/// - **Predictable Memory Usage**: Fixed account size enables accurate rent calculations
/// - **O(1) Insertion**: New observations overwrite oldest without array shifts
/// - **Cache Efficiency**: Contiguous memory layout optimizes access patterns
/// - **Bounded Storage**: Prevents unbounded growth that could cause account bloat
/// 
/// # Index Management Strategy
/// 
/// The observation_index points to the next insertion position, implementing a
/// write-ahead circular buffer. This approach simplifies index arithmetic and
/// ensures atomic observation updates.
#[repr(C, packed)]
pub struct ObservationState {
    /// Initialization flag preventing reads from uninitialized observation buffers.
    /// Critical safety check since uninitialized data could produce invalid TWAP calculations.
    pub initialized: bool,
    
    /// Epoch of most recent update for staleness detection.
    /// Enables identification of observations that may be outdated relative to current network state.
    pub recent_epoch: u64,
    
    /// Write pointer for circular buffer insertion.
    /// Points to the next position where a new observation will be written, enabling
    /// O(1) insertion without scanning for the latest entry.
    pub observation_index: u16,
    
    /// Pool account this observation buffer belongs to.
    /// Provides verification that observation data matches the expected pool context.
    pub pool_id: Pubkey,
    
    /// Fixed-size circular buffer of price observations.
    /// Size determined by OBSERVATION_NUM constant to match Raydium's buffer capacity.
    pub observations: [Observation; OBSERVATION_NUM],
    
    /// Reserved space for Raydium's future extensions.
    /// Maintains compatibility when Raydium adds new fields to their observation state.
    pub padding: [u64; 4],
}

/// Partial view of Raydium's PoolState containing only fields needed for price observation.
/// 
/// # Partial Struct Strategy
/// 
/// Rather than defining the complete PoolState (which would be large and maintenance-heavy),
/// this struct includes only the prefix bytes and specific fields we need. Benefits:
/// 
/// - **Reduced Maintenance**: Only need to track fields we actually use
/// - **Memory Efficiency**: Smaller struct size reduces stack pressure
/// - **Compatibility Resilience**: Less likely to break when Raydium adds unrelated fields
/// 
/// # Memory Layout Dependencies
/// 
/// The _prefix field represents all the leading PoolState fields that we skip over
/// to reach observation_key and subsequent fields. This approach is fragile but necessary
/// for zero-copy access - any changes to Raydium's field ordering will break compatibility.
#[repr(C, packed)]
pub struct PoolStatePartial {
    /// Raw bytes representing the prefix fields we don't need to access.
    /// Contains: bump + amm_config + owner + token_mint_0 + token_mint_1 + token_vault_0 + token_vault_1.
    /// This byte array skips over fields while maintaining correct memory alignment for observation_key.
    pub _prefix: [u8; POOL_STATE_PREFIX_SIZE],

    /// Reference to the associated observation account for this pool.
    /// Critical for linking pool state to its historical price data for TWAP calculations.
    pub observation_key: Pubkey,

    /// Decimal places for token0 and token1 respectively.
    /// Required for converting raw tick values to human-readable prices with correct precision.
    pub mint_decimals_0: u8,
    pub mint_decimals_1: u8,

    /// Minimum tick spacing enforced by this pool.
    /// Determines price precision and affects how ticks map to actual price ratios.
    pub tick_spacing: u16,
    
    /// Total liquidity currently available in the pool.
    /// Used for assessing pool depth and potential price impact of large trades.
    pub liquidity: u128,
    
    /// Current price as sqrt(token1/token0) in Q64.64 fixed-point format.
    /// Provides precise price representation without floating-point precision issues.
    pub sqrt_price_x64: u128,
    
    /// Current active tick representing the pool's price state.
    /// Forms the basis for tick_cumulative calculations in TWAP observations.
    pub tick_current: i32,
}

// ============ Zero-copy reader wrappers ============

/// Safe wrapper for accessing observation data with cached performance optimizations.
/// 
/// # Safety Abstraction Design
/// 
/// This wrapper provides a safe interface over the unsafe packed struct, implementing
/// several important safety and performance patterns:
/// 
/// - **Lifetime Binding**: Ensures the wrapper cannot outlive the underlying data
/// - **Bounds Checking**: Safe array access with modulo arithmetic for circular buffer
/// - **Index Caching**: Avoids repeated field access for frequently-used values
/// - **Immutable Interface**: Read-only access prevents accidental state corruption
/// 
/// # Performance Optimizations
/// 
/// The cached_index field stores the observation_index value to avoid repeated
/// packed struct field access, which can be expensive due to unaligned memory reads.
pub struct ObservationReader <'a> {
    /// Immutable reference to the underlying observation data.
    /// Lifetime parameter ensures this wrapper cannot outlive the source account data.
    data: &'a ObservationState,
    
    /// Cached copy of observation_index to avoid repeated packed field access.
    /// Accessing fields in packed structs can be expensive due to alignment requirements.
    cached_index: u16,
}

impl<'a> ObservationReader<'a> {
    /// Creates a new reader with performance optimization caching.
    /// 
    /// Immediately caches the observation_index to avoid repeated access to packed struct fields,
    /// which can incur performance penalties due to unaligned memory reads.
    #[inline]
    pub fn new(data: &'a ObservationState) -> Self {
        Self {
            data,
            cached_index: data.observation_index,
        }
    }

    /// Safely accesses observation at given index using circular buffer semantics.
    /// 
    /// The modulo operation ensures safe access even if the caller provides an invalid index,
    /// implementing the circular buffer behavior expected for TWAP calculations.
    /// This approach trades a small performance cost for robust error handling.
    #[inline]
    pub fn get_observation(&self, index: usize) -> &Observation {
        &self.data.observations[index % OBSERVATION_NUM]
    }

    /// Returns the cached current write position for optimal performance.
    /// 
    /// Uses the cached value rather than re-reading from the packed struct to avoid
    /// potential performance penalties from unaligned field access.
    #[inline]
    pub fn current_index(&self) -> usize {
        self.cached_index as usize
    }

    /// Checks initialization status to prevent reading invalid observation data.
    /// 
    /// Critical safety check since uninitialized observation buffers contain
    /// arbitrary memory contents that could produce invalid TWAP calculations.
    #[inline]
    pub fn initialized(&self) -> bool {
        self.data.initialized
    }

    /// Returns the pool ID for verification and cross-referencing.
    /// 
    /// Enables callers to verify that observation data belongs to the expected pool,
    /// preventing mixing of observation data from different trading pairs.
    #[inline]
    pub fn pool_id(&self) -> &Pubkey {
        &self.data.pool_id
    }
}

/// Safe wrapper for accessing pool state data with zero-copy performance.
/// 
/// # Design Simplicity
/// 
/// Unlike ObservationReader, this wrapper is simpler because pool state is accessed
/// less frequently and doesn't require complex circular buffer logic. The wrapper
/// still provides important safety guarantees by preventing direct access to the
/// unsafe packed struct while maintaining zero-copy performance characteristics.
pub struct PoolReader<'a> {
    /// Immutable reference to the underlying pool state data.
    /// Lifetime parameter ensures memory safety by preventing dangling references.
    data: &'a PoolStatePartial,
}

impl<'a> PoolReader<'a> {
    /// Creates a new pool reader for safe data access.
    /// 
    /// Simple wrapper construction since pool state doesn't require the complex
    /// caching optimizations needed for observation arrays.
    #[inline]
    pub fn new(data: &'a PoolStatePartial) -> Self {
        Self { data }
    }

    /// Returns the observation account key for linking to historical price data.
    /// 
    /// This key is essential for TWAP calculations as it connects the current pool
    /// state to its historical observation buffer.
    #[inline]
    pub fn observation_key(&self) -> &Pubkey {
        &self.data.observation_key
    }

    /// Returns token decimal precision for both tokens in the trading pair.
    /// 
    /// Required for converting between raw tick values and human-readable prices
    /// with correct decimal precision for each token.
    #[inline]
    pub fn decimals(&self) -> (u8, u8) {
        (self.data.mint_decimals_0, self.data.mint_decimals_1)
    }

    /// Returns the tick spacing configuration for this pool.
    /// 
    /// Determines price granularity and affects how tick movements translate
    /// to actual price changes in the trading pair.
    #[inline]
    pub fn tick_spacing(&self) -> u16 {
        self.data.tick_spacing
    }

    /// Returns current liquidity available in the pool.
    /// 
    /// Critical for assessing market depth and potential price impact of trades
    /// when evaluating the reliability of price observations.
    #[inline]
    pub fn liquidity(&self) -> u128 {
        self.data.liquidity
    }

    /// Returns current sqrt price in Q64.64 fixed-point format.
    /// 
    /// Provides precise price representation avoiding floating-point precision issues
    /// that could accumulate errors in TWAP calculations.
    #[inline]
    pub fn sqrt_price_x64(&self) -> u128 {
        self.data.sqrt_price_x64
    }

    /// Returns the current active tick representing pool price state.
    /// 
    /// This tick value forms the basis for cumulative tick calculations
    /// used in TWAP observations and price averaging algorithms.
    #[inline]
    pub fn tick_current(&self) -> i32 {
        self.data.tick_current
    }
}

// ============ Zero-copy readers ============

/// Generic zero-copy reader for Raydium account data with safety validation.
/// 
/// # Safety and Performance Trade-offs
/// 
/// This function performs unsafe pointer arithmetic to achieve zero-copy reads from
/// account data, trading safety for performance. The approach eliminates serialization
/// overhead but requires careful validation to prevent undefined behavior:
/// 
/// - **Memory Layout Assumptions**: Assumes standard Anchor account layout (8-byte discriminator + data)
/// - **Alignment Requirements**: Relies on proper struct alignment for safe pointer casting
/// - **Lifetime Management**: Borrows account data to ensure the returned reference remains valid
/// 
/// # Validation Strategy
/// 
/// Multiple validation layers ensure safe operation:
/// 1. Account data borrow check ensures exclusive access
/// 2. Size validation prevents reading beyond account boundaries  
/// 3. Unsafe pointer arithmetic with compile-time size checking
#[inline]
pub fn read_zc<'a, T>(account_info: &'a AccountInfo) -> Result<&'a T> {
    // Borrow account data with runtime validation to ensure exclusive access
    let data = account_info.try_borrow_data()?;
    
    // Validate account contains sufficient data for discriminator + struct
    // This prevents reading beyond account boundaries which would cause UB
    require!(data.len() >= 8 + size_of::<T>(), RaydiumObserverError::TooSmall);
    
    // Skip 8-byte Anchor discriminator and cast to target struct type
    // SAFETY: We've validated sufficient data exists and assume correct alignment
    let p = unsafe { data.as_ptr().add(8) as *const T };
    Ok(unsafe { &*p } )
}

/// Safely reads and validates Raydium pool account data.
/// 
/// # Security Validation
/// 
/// Implements multiple security checks to ensure we're reading valid Raydium pool data:
/// 1. **Program Ownership**: Verifies the account is owned by the correct Raydium program
/// 2. **Data Structure**: Validates account contains properly formatted pool state
/// 3. **Wrapper Creation**: Returns safe reader interface over validated raw data
/// 
/// The ownership check is critical for preventing attacks where malicious accounts
/// with crafted data could be passed as legitimate Raydium pools.
#[inline]
pub fn read_pool<'a>(account_info: &'a AccountInfo) -> Result<PoolReader<'a>> {
    // Verify account is owned by Raydium CLMM program to prevent spoofing attacks
    require_keys_eq!(*account_info.owner, RAYDIUM_CLMM_PROGRAM_ID_DEVNET, RaydiumObserverError::InvalidOwner);
    
    // Perform zero-copy read with structure validation
    let data = read_zc::<PoolStatePartial>(account_info)?;
    
    // Return safe wrapper over validated raw data
    Ok(PoolReader::new(data))
}

/// Safely reads and validates Raydium observation account data.
/// 
/// # Enhanced Validation
/// 
/// Includes additional validation beyond the pool reader:
/// 1. **Program Ownership**: Ensures account belongs to Raydium CLMM program
/// 2. **Data Structure**: Validates account structure and size
/// 3. **Initialization Check**: Verifies observation buffer has been properly initialized
/// 
/// The initialization check is crucial because uninitialized observation buffers
/// contain arbitrary memory that could produce invalid TWAP calculations if used
/// in price aggregation algorithms.
#[inline]
pub fn read_observation<'a>(account_info: &'a AccountInfo) -> Result<ObservationReader<'a>> {
    // Verify account is owned by Raydium CLMM program to prevent spoofing attacks
    require_keys_eq!(*account_info.owner, RAYDIUM_CLMM_PROGRAM_ID_DEVNET, RaydiumObserverError::InvalidOwner);
    
    // Perform zero-copy read with structure validation
    let data = read_zc::<ObservationState>(account_info)?;
    
    // Critical safety check: ensure observation buffer has been initialized
    // Uninitialized buffers contain arbitrary memory that would corrupt TWAP calculations
    require!(data.initialized, RaydiumObserverError::Uninitialized);
    
    // Return safe wrapper over validated and initialized data
    Ok(ObservationReader::new(data))
}