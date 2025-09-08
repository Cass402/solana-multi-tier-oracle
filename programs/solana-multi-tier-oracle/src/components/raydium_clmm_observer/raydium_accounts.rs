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
use core::ptr;
use core::mem::{size_of};
use crate::components::raydium_clmm_observer::raydium_constants::{OBSERVATION_NUM, RAYDIUM_CLMM_PROGRAM_ID_DEVNET, OBSERVATION_SEED};
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

/// Lightweight proxy for safe access to packed Observation data via unsafe pointers.
/// 
/// # Safety Abstraction Strategy
/// 
/// This proxy wraps raw pointer access to packed structs, providing a safe interface
/// while maintaining zero-copy performance. The design isolates unsafe operations
/// within controlled methods that handle alignment and dereferencing correctly.
/// 
/// # Copy Semantics
/// 
/// Implements Copy to enable efficient passing without move semantics overhead,
/// critical for high-frequency price data access in TWAP calculations. The proxy
/// itself is just a pointer wrapper, making copying trivially cheap.
#[derive(Clone, Copy)]
pub struct ObservationProxy {
    /// Raw pointer to packed Observation struct in account data.
    /// Must be valid for the lifetime of the containing ObservationReader to prevent UAF.
    data: *const Observation,
}

impl ObservationProxy {
    /// Extract block timestamp using unaligned read to handle packed struct memory layout.
    /// 
    /// # Alignment Safety
    /// 
    /// Uses read_unaligned because packed structs don't guarantee field alignment,
    /// and direct field access could cause alignment faults on some architectures.
    /// The addr_of! macro creates a pointer without intermediate references,
    /// preventing undefined behavior from creating unaligned references.
    #[inline(always)]
    pub fn block_timestamp(self) -> u32 {
        unsafe { ptr::read_unaligned(ptr::addr_of!((*self.data).block_timestamp)) }
    }

    /// Extract tick cumulative value using safe unaligned pointer arithmetic.
    /// 
    /// # Memory Layout Considerations
    /// 
    /// The packed struct layout means tick_cumulative may not be aligned to its
    /// natural boundary, requiring unaligned reads. This pattern is essential
    /// for compatibility with Raydium's C-style packed struct definitions.
    #[inline(always)]
    pub fn tick_cumulative(self) -> i64 {
        unsafe { ptr::read_unaligned(ptr::addr_of!((*self.data).tick_cumulative)) }
    }
}

/// Safe reader wrapper for ObservationState with managed lifetime and cached metadata.
/// 
/// # Lifetime Management Strategy
/// 
/// Holds a borrowing reference to account data to ensure the underlying memory
/// remains valid for the reader's lifetime. This prevents use-after-free bugs
/// that could occur if the account data is deallocated while pointers remain active.
/// 
/// # Performance Optimizations
/// 
/// - Caches frequently accessed observation_index to avoid repeated unsafe reads
/// - Uses direct pointer arithmetic for O(1) observation access within circular buffer
/// - Inlines critical path methods to eliminate function call overhead
/// 
/// # Memory Safety Architecture
/// 
/// The reader maintains both a reference to keep data alive and a typed pointer
/// for efficient access. This dual approach ensures safety while maximizing performance
/// for time-critical TWAP calculations.
pub struct ObservationReader <'a> {
    /// Borrowed reference keeping account data alive for reader lifetime.
    /// Prevents the account data from being deallocated while we hold pointers into it.
    _data_ref: std::cell::Ref<'a, &'a mut [u8]>,
    
    /// Typed pointer to ObservationState for zero-copy field access.
    /// Valid as long as _data_ref remains alive, ensuring no dangling pointer access.
    data: *const ObservationState,
    
    /// Cached observation index to avoid repeated unsafe pointer reads.
    /// Updated only during construction since index changes require account updates.
    cached_index: u16,
}

impl<'a> ObservationReader<'a> {
    /// Construct reader with validation and pointer initialization for zero-copy access.
    /// 
    /// # Validation Strategy
    /// 
    /// Performs size validation before creating pointers to prevent buffer overflows
    /// when accessing ObservationState fields. The 8-byte offset accounts for Anchor's
    /// discriminator prefix that precedes all account data.
    /// 
    /// # Caching Rationale
    /// 
    /// Immediately caches the observation_index to avoid repeated unsafe reads during
    /// TWAP calculations. Since index updates require full account updates, this
    /// caching approach is safe and provides meaningful performance benefits.
    #[inline]
    pub fn new_ptr(account_info: &'a AccountInfo) -> Result<Self> {
        let data = account_info.try_borrow_data()?;
        
        // Validate account has sufficient size for discriminator + ObservationState
        // Prevents buffer overflows during pointer arithmetic and field access
        require!(data.len() >= 8 + size_of::<ObservationState>(), RaydiumObserverError::TooSmall);

        // Skip 8-byte Anchor discriminator to access actual account data
        let ptr = unsafe { data.as_ptr().add(8) as *const ObservationState };

        let reader = Self {
            _data_ref: data,
            data: ptr,
            // Cache index immediately to avoid repeated unsafe reads during TWAP operations
            cached_index: unsafe { ptr::read_unaligned(ptr::addr_of!((*ptr).observation_index)) },
        };

        Ok(reader)
    }

    /// Access individual observation using modular arithmetic for circular buffer traversal.
    /// 
    /// # Circular Buffer Safety
    /// 
    /// Uses modular arithmetic (index % OBSERVATION_NUM) to ensure array bounds safety
    /// even with invalid input indices. This prevents buffer overflows while providing
    /// efficient O(1) access to any observation in the circular buffer.
    /// 
    /// # Pointer Arithmetic Strategy
    /// 
    /// Calculates observation addresses using direct pointer arithmetic rather than
    /// array indexing to maintain zero-copy performance. The unsafe pointer operations
    /// are contained within this method, providing a safe interface to callers.
    #[inline]
    pub fn get_observation(&self, index: usize) -> ObservationProxy {
        // Get base address of observations array within ObservationState
        let observation_0 = unsafe { ptr::addr_of!((*self.data).observations) as *const Observation };
        
        // Use modular arithmetic to ensure bounds safety in circular buffer access
        let ptr = unsafe { observation_0.add(index % OBSERVATION_NUM) };

        ObservationProxy { data: ptr }
    }

    /// Return cached current index for efficient circular buffer navigation.
    /// 
    /// # Caching Benefits
    /// 
    /// Returns pre-cached index value to avoid unsafe pointer reads during TWAP
    /// calculations where index access frequency is high. The cached value remains
    /// valid since index updates require account state changes visible to our reader.
    #[inline]
    pub fn current_index(&self) -> usize {
        self.cached_index as usize
    }

    /// Check initialization state using unaligned read for packed struct compatibility.
    /// 
    /// # Initialization Safety
    /// 
    /// Critical validation to prevent TWAP calculations on uninitialized observation
    /// buffers that could contain arbitrary data. Uses unaligned read to handle
    /// potential alignment issues in packed struct layout.
    #[inline]
    pub fn initialized(&self) -> bool {
        unsafe { ptr::read_unaligned(ptr::addr_of!((*self.data).initialized)) }
    }

    /// Extract pool identifier for observation-to-pool relationship verification.
    /// 
    /// # Cross-Account Validation
    /// 
    /// Enables verification that observation data corresponds to the expected pool,
    /// preventing cross-pool data contamination in multi-pool oracle operations.
    /// Essential for maintaining data integrity in complex DeFi integrations.
    #[inline]
    pub fn pool_id(&self) -> Pubkey {
        unsafe { ptr::read_unaligned(ptr::addr_of!((*self.data).pool_id)) }
    }
}

/// Safe reader wrapper for PoolState with managed lifetime and zero-copy field access.
/// 
/// # Design Rationale
/// 
/// Similar to ObservationReader but optimized for PoolState access patterns.
/// Provides safe access to critical pool metadata needed for price calculations
/// while maintaining zero-copy performance characteristics.
/// 
/// # Memory Management
/// 
/// Uses the same lifetime management strategy as ObservationReader to ensure
/// underlying account data remains valid throughout the reader's usage period.
pub struct PoolReader<'a> {
    /// Borrowed reference keeping account data alive for reader lifetime.
    _data_ref: std::cell::Ref<'a, &'a mut [u8]>,
    
    /// Typed pointer to PoolStatePartial for efficient field access.
    base: *const PoolStatePartial,
}

impl<'a> PoolReader<'a> {
    /// Construct pool reader with validation and pointer setup for zero-copy access.
    /// 
    /// # Size Validation Strategy
    /// 
    /// Validates account size against PoolStatePartial rather than full PoolState
    /// since we only access a subset of fields. This approach is more resilient
    /// to Raydium adding fields we don't use at the end of their struct.
    #[inline]
    pub fn new_ptr(account_info: &'a AccountInfo) -> Result<Self> {
        let data = account_info.try_borrow_data()?;
        
        // Ensure sufficient size for discriminator + PoolStatePartial fields
        require!(data.len() >= 8 + size_of::<PoolStatePartial>(), RaydiumObserverError::TooSmall);

        // Skip Anchor discriminator to access pool state data
        let ptr = unsafe { data.as_ptr().add(8) as *const PoolStatePartial };

        Ok(Self {
            _data_ref: data,
            base: ptr,
        })
    }

    /// Extract observation account key for pool-observation linkage verification.
    /// 
    /// # Cross-Account Integrity
    /// 
    /// Critical for ensuring observation data matches the expected pool context.
    /// Used in verification functions to prevent observation account spoofing
    /// or incorrect pool-observation associations.
    #[inline]
    pub fn observation_key(&self) -> Pubkey {
        unsafe { ptr::read_unaligned(ptr::addr_of!((*self.base).observation_key)) }
    }

    /// Return token decimal configuration for price precision calculations.
    /// 
    /// # Price Conversion Context
    /// 
    /// Token decimals are essential for converting raw tick values to human-readable
    /// prices with correct precision. Different tokens can have different decimal
    /// places (e.g., USDC=6, ETH=18), requiring this metadata for accurate pricing.
    #[inline]
    pub fn decimals(&self) -> (u8, u8) {
        let decimal_0 = unsafe { ptr::read_unaligned(ptr::addr_of!((*self.base).mint_decimals_0)) };
        let decimal_1 = unsafe { ptr::read_unaligned(ptr::addr_of!((*self.base).mint_decimals_1)) };

        (decimal_0, decimal_1)
    }

    /// Get tick spacing configuration affecting price precision granularity.
    /// 
    /// # Price Precision Impact
    /// 
    /// Tick spacing determines the minimum price increments possible in the pool.
    /// Smaller spacing allows finer price precision but increases computational
    /// overhead for large price movements. Critical for accurate TWAP calculations.
    #[inline]
    pub fn tick_spacing(&self) -> u16 {
        unsafe{ ptr::read_unaligned(ptr::addr_of!((*self.base).tick_spacing)) }
    }

    /// Extract current liquidity for manipulation risk assessment.
    /// 
    /// # Market Depth Analysis
    /// 
    /// Pool liquidity indicates how much capital is available at current prices.
    /// Low liquidity pools are more susceptible to price manipulation and may
    /// require higher confidence thresholds in oracle risk assessments.
    #[inline]
    pub fn liquidity(&self) -> u128 {
        unsafe { ptr::read_unaligned(ptr::addr_of!((*self.base).liquidity)) }
    }

    /// Get current sqrt price in Q64.64 fixed-point format for precise calculations.
    /// 
    /// # Fixed-Point Precision Strategy
    /// 
    /// Q64.64 format provides sufficient precision for financial calculations
    /// while avoiding floating-point precision issues. The sqrt representation
    /// enables efficient computation of price ratios and tick conversions.
    #[inline]
    pub fn sqrt_price_x64(&self) -> u128 {
        unsafe { ptr::read_unaligned(ptr::addr_of!((*self.base).sqrt_price_x64)) }
    }

    /// Extract current tick representing the pool's active price level.
    /// 
    /// # Price State Context
    /// 
    /// The current tick serves as the reference point for TWAP calculations
    /// and indicates the pool's instantaneous price state. Forms the basis
    /// for tick_cumulative updates in new observations.
    #[inline]
    pub fn tick_current(&self) -> i32 {
        unsafe { ptr::read_unaligned(ptr::addr_of!((*self.base).tick_current)) }
    }
}

// ============ Zero-copy readers ============

/// Generic zero-copy pointer extraction with validation for any Anchor account type.
/// 
/// # Generic Design Strategy
/// 
/// Provides a reusable pattern for zero-copy account access across different account
/// types. The generic approach eliminates code duplication while maintaining type
/// safety for the resulting pointers.
/// 
/// # Validation Approach
/// 
/// Performs size validation before pointer creation to prevent buffer overflows.
/// The 8-byte offset accounts for Anchor's account discriminator that precedes
/// all account data in the Solana account model.
#[inline]
pub fn read_zc_ptr<T>(account_info: &AccountInfo) -> Result<*const T> {
    let data = account_info.try_borrow_data()?;
    
    // Validate sufficient space for discriminator + target struct
    require!(data.len() >= 8 + size_of::<T>(), RaydiumObserverError::TooSmall);

    // Skip 8-byte Anchor discriminator to access actual account data
    let ptr = unsafe { data.as_ptr().add(8) as *const T };

    Ok(ptr)
}

/// Create validated PoolReader with ownership verification for cross-program security.
/// 
/// # Cross-Program Security Model
/// 
/// Validates account ownership before creating reader to prevent spoofing attacks
/// where malicious accounts mimic Raydium pool structures. Ownership verification
/// is fundamental to cross-program invocation security in Solana.
/// 
/// # Performance Strategy
/// 
/// Combines ownership validation with reader creation in single function to
/// reduce call overhead while maintaining security. Inlined for zero-cost
/// abstraction in hot code paths.
#[inline]
pub fn read_pool<'a>(account_info: &'a AccountInfo, program_id: &Pubkey) -> Result<PoolReader<'a>> {
    // Verify account is owned by legitimate Raydium program to prevent spoofing
    require_keys_eq!(*account_info.owner, *program_id, RaydiumObserverError::InvalidOwner);
    PoolReader::new_ptr(account_info)
}

/// Create validated ObservationReader with comprehensive security checks.
/// 
/// # Multi-Layer Validation Strategy
/// 
/// Implements defense-in-depth by combining:
/// 1. Ownership verification to prevent account spoofing
/// 2. Initialization checks to avoid reading uninitialized data
/// 3. Size validation within the reader constructor
/// 
/// This layered approach ensures observation data integrity even if individual
/// checks are bypassed or fail due to edge case conditions.
/// 
/// # Oracle Security Context
/// 
/// Particularly critical for oracle operations where corrupted price data could
/// propagate through the system and enable economic exploits. The validation
/// ensures only legitimate, initialized Raydium observations are processed.
#[inline]
pub fn read_observation<'a>(account_info: &'a AccountInfo, program_id: &Pubkey) -> Result<ObservationReader<'a>> {
    // First layer: Verify account ownership to prevent spoofing attacks
    require_keys_eq!(*account_info.owner, *program_id, RaydiumObserverError::InvalidOwner);

    let reader = ObservationReader::new_ptr(account_info)?;
    
    // Second layer: Ensure observation buffer is properly initialized
    // Prevents TWAP calculations on arbitrary uninitialized memory
    require!(reader.initialized(), RaydiumObserverError::Uninitialized);

    Ok(reader)
}

/// Comprehensive pool-observation relationship verification with PDA validation.
/// 
/// # Account Relationship Security
/// 
/// Implements multi-point verification to ensure authentic pool-observation linkage:
/// 1. **PDA Derivation**: Verifies observation account matches expected derivation
/// 2. **Ownership Validation**: Confirms pool account is owned by Raydium program
/// 3. **Cross-Reference Check**: Validates pool's observation_key matches provided account
/// 
/// # Attack Prevention Strategy
/// 
/// This comprehensive verification prevents several attack vectors:
/// - **PDA Spoofing**: Fake observation accounts with similar addresses
/// - **Pool Substitution**: Using legitimate observations with wrong pools
/// - **Cross-Pool Contamination**: Mixing price data between different pools
/// 
/// # Network Configuration
/// 
/// Currently hardcoded to DEVNET program ID for development safety.
/// Production deployments should parameterize this based on network detection.
#[inline]
pub fn verify_observation_pda_and_read_pool<'a>(
    pool_account_info: &'a AccountInfo,
    observation_account_info: &'a AccountInfo,
    program_id: &Pubkey,
) -> Result<PoolReader<'a>> {
    // Derive expected observation account address using Raydium's PDA scheme
    // Seeds: "observation" + pool_pubkey ensures unique observation per pool
    let (derived, _) = Pubkey::find_program_address(&[
        OBSERVATION_SEED.as_ref(),
        pool_account_info.key.as_ref(),
    ],
        &RAYDIUM_CLMM_PROGRAM_ID_DEVNET  // Network-aware program ID selection
    );
    
    // Verify provided observation account matches expected PDA derivation
    require_keys_eq!(derived, *observation_account_info.key, RaydiumObserverError::BadPda);

    // Create validated pool reader with ownership checks
    let pool = read_pool(pool_account_info, program_id)?;
    
    // Cross-validate that pool's observation_key references the provided observation account
    // Prevents pool-observation mixups that could corrupt TWAP calculations
    require_keys_eq!(pool.observation_key(), *observation_account_info.key, RaydiumObserverError::PoolMismatch);
    
    Ok(pool)
}
