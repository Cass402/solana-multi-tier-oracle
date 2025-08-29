use crate::error::StateError;
use crate::utils::constants::{MAX_PRICE_FEEDS, MAX_HISTORICAL_CHUNKS, MAX_LP_CONCENTRATION};
use crate::state::price_feed::PriceFeed;
use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

/// Core oracle state managing price aggregation across multiple data sources.
/// 
/// # Architecture Design Rationale
/// 
/// This struct uses Anchor's zero-copy pattern to minimize heap allocations and enable
/// direct memory mapping for high-frequency price updates. The layout is carefully designed
/// to balance several competing concerns:
/// 
/// - **Memory Efficiency**: Fixed-size arrays avoid Vec overhead and enable predictable
///   account size calculations for rent exemption
/// - **Cache Performance**: Fields are ordered by access frequency and aligned to 
///   minimize cache misses during price aggregation operations
/// - **Upgrade Safety**: Reserved space and version tracking enable backward-compatible
///   schema evolution without data migration
/// - **MEV Resistance**: Manipulation detection thresholds and LP concentration limits
///   protect against oracle manipulation attacks
/// 
/// The zero-copy approach is critical for Solana's compute unit constraints, as it
/// eliminates serialization overhead that would otherwise consume significant CU budget
/// during frequent price updates.
#[account(zero_copy)]
#[derive(InitSpace)]
#[repr(C)]
pub struct OracleState {
    /// Governance authority with upgrade and emergency powers.
    /// Separated from operational updates to enable secure key management practices.
    pub authority: Pubkey,
    
    /// Schema version for backward-compatible upgrades.
    /// Enables graceful handling of account data from different program versions.
    pub version: Version,
    
    /// Bitfield for operational state flags.
    /// Compact representation saves space while enabling atomic state transitions.
    pub flags: StateFlags,

    /// Most recent aggregated price with confidence interval.
    /// Positioned early in struct for optimal cache locality during frequent reads.
    pub current_price: PriceData,
    
    /// Unix timestamp of last successful price update.
    /// Used for staleness detection and circuit breaker logic.
    pub last_update: u64,
    
    /// Fixed array of price feed sources to avoid heap allocation.
    /// Size chosen as power-of-2 for optimal memory alignment and cache performance.
    pub price_feeds: [PriceFeed; MAX_PRICE_FEEDS],

    /// TWAP calculation window in seconds.
    /// Balances responsiveness vs manipulation resistance.
    pub twap_window: u16,
    
    /// Current position in circular buffer for historical data.
    /// Enables efficient O(1) historical data management without shifts.
    pub current_chunk_index: u16,
    
    /// Maximum entries per historical chunk before rotation.
    /// Tuned to balance storage costs with historical depth requirements.
    pub max_chunk_size: u16,

    /// Minimum confidence threshold for price acceptance (basis points).
    /// Rejects low-quality data that could compromise aggregation accuracy.
    pub confidence_threshold: u16,
    
    /// Maximum manipulation score before triggering circuit breaker.
    /// Protects against coordinated attacks on multiple price sources.
    pub manipulation_threshold: u16,

    /// Number of currently active feeds for efficient iteration.
    /// Avoids scanning entire array when only subset is active.
    pub active_feed_count: u8,
    
    /// PDA bump seed for deterministic address derivation.
    /// Stored to avoid recomputation during account validation.
    pub bump: u8,

    /// References to historical price data chunks stored in separate accounts.
    /// Enables unbounded historical data while respecting account size limits.
    pub historical_chunks: [Pubkey; MAX_HISTORICAL_CHUNKS],

    /// Explicit padding to ensure deterministic struct layout.
    /// Prevents compiler-dependent alignment issues across builds.
    pub _padding: [u8; 4],

    /// Reserved space for future schema additions without breaking changes.
    /// Sized to accommodate common future fields while maintaining rent exemption.
    pub reserved: [u64; 8],
}

/// Compact bitfield for oracle operational state management.
/// 
/// # Design Philosophy
/// 
/// Uses a transparent wrapper around u32 to provide type-safe flag operations while
/// maintaining binary compatibility with raw integer storage. This approach offers
/// several advantages over enum-based state management:
/// 
/// - **Atomic Operations**: Multiple flags can be set/cleared in single operation
/// - **Space Efficiency**: 32 flags consume only 4 bytes vs separate boolean fields
/// - **Forward Compatibility**: New flags can be added without breaking existing data
/// - **Performance**: Bitwise operations are faster than multiple boolean checks
/// 
/// The transparent repr ensures zero-cost abstractions - compiled code operates
/// directly on the underlying u32 without wrapper overhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Default, InitSpace)]
#[repr(transparent)]
pub struct StateFlags(u32);

impl StateFlags {
    /// Flag definitions using binary literals for clarity and audit trail.
    /// Each flag represents a distinct operational mode with specific security implications.
    
    /// Halts price updates when manipulation is detected.
    /// Critical safety mechanism to prevent oracle attacks from propagating.
    pub const CIRCUIT_BREAKER_ENABLED: Self = Self(0b0000_0001);
    
    /// Disables all non-critical operations during security incidents.
    /// Preserves core functionality while limiting attack surface.
    pub const EMERGENCY_MODE: Self          = Self(0b0000_0010);
    
    /// Prevents program upgrades during critical operations.
    /// Ensures operational stability during high-stakes periods.
    pub const UPGRADE_LOCKED: Self          = Self(0b0000_0100);
    
    /// Scheduled downtime for system maintenance.
    /// Distinguishes planned vs emergency outages for monitoring.
    pub const MAINTENANCE_MODE: Self        = Self(0b0000_1000);
    
    /// Enables time-weighted average price calculations.
    /// Adds computational overhead but improves manipulation resistance.
    pub const TWAP_ENABLED: Self            = Self(0b0001_0000);

    /// Bitmask defining all currently valid flag positions.
    /// Used for forward-compatible deserialization that ignores unknown flags.
    pub const VALID_MASK: u32 = Self::CIRCUIT_BREAKER_ENABLED.0
        | Self::EMERGENCY_MODE.0
        | Self::UPGRADE_LOCKED.0
        | Self::MAINTENANCE_MODE.0
        | Self::TWAP_ENABLED.0;

    /// Creates empty flag set with all flags disabled.
    /// const fn enables compile-time initialization for static instances.
    #[inline(always)] 
    pub const fn new() -> Self { Self(0) }

    /// Tests if any bits from the specified flag are set.
    /// Efficient single-instruction check using bitwise AND.
    #[inline(always)] 
    pub fn has(self, flag: Self) -> bool { 
        (self.0 & flag.0) != 0 
    }

    /// Sets the specified flag bits using bitwise OR.
    /// Preserves existing flags while enabling new ones.
    #[inline(always)] 
    pub fn set(&mut self, flag: Self) {
        self.0 |= flag.0; 
    }

    /// Clears the specified flag bits using bitwise AND with negation.
    /// Preserves other flags while disabling target ones.
    #[inline(always)] 
    pub fn clear(&mut self, flag: Self) { 
        self.0 &= !flag.0; 
    }

    /// Toggles the specified flag bits using bitwise XOR.
    /// Useful for state transitions that depend on current state.
    #[inline(always)] 
    pub fn toggle(&mut self, flag: Self)  { 
        self.0 ^= flag.0; 
    }

    /// Conditionally sets or clears flag based on boolean parameter.
    /// Enables clean conditional flag management without branching in caller.
    #[inline(always)] 
    pub fn set_to(&mut self, flag: Self, on: bool) {
        if on { self.set(flag) } else { self.clear(flag) }
    }

    /// Semantic convenience methods for common flag checks.
    /// These provide better API ergonomics while compiling to identical assembly.
    
    #[inline(always)] 
    pub fn is_circuit_breaker_enabled(self) -> bool { 
        self.has(Self::CIRCUIT_BREAKER_ENABLED) 
    }

    #[inline(always)] 
    pub fn is_emergency_mode(self) -> bool { 
        self.has(Self::EMERGENCY_MODE) 
    }

    #[inline(always)] 
    pub fn is_upgrade_locked(self) -> bool { 
        self.has(Self::UPGRADE_LOCKED) 
    }

    #[inline(always)] 
    pub fn is_maintenance_mode(self) -> bool { 
        self.has(Self::MAINTENANCE_MODE) 
    }

    #[inline(always)] 
    pub fn is_twap_enabled(self) -> bool { 
        self.has(Self::TWAP_ENABLED) 
    }

    /// Serialization helpers for account I/O operations.
    
    /// Extracts raw u32 value for storage in account data.
    /// const fn enables compile-time evaluation where possible.
    #[inline(always)] 
    pub const fn as_u32(self) -> u32 { 
        self.0 
    }

    /// Creates StateFlags from raw u32, filtering unknown flag bits.
    /// Forward-compatible deserialization ignores flags from newer program versions,
    /// preventing crashes when reading data written by future versions.
    #[inline(always)] 
    pub const fn from_u32_truncate(value: u32) -> Self {
        Self(value & Self::VALID_MASK)
    }
}

/// Semantic versioning for oracle schema evolution.
/// 
/// Enables backward-compatible account data migrations when program logic changes.
/// The explicit padding ensures consistent struct layout across different architectures
/// and compiler versions, preventing silent data corruption during upgrades.
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, InitSpace)]
#[repr(C)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
    /// Explicit padding prevents compiler-dependent alignment.
    /// Critical for deterministic account layout across deployment environments.
    pub _padding: u8,
}

/// Standardized price representation with confidence intervals.
/// 
/// # Design Considerations
/// 
/// - **Signed Price**: i64 accommodates negative prices for derivatives/spreads
/// - **Confidence Interval**: u64 provides sufficient precision for basis point accuracy
/// - **Unix Timestamp**: Standard format enables easy integration with external systems
/// - **Scientific Notation**: expo field supports assets across vastly different price ranges
/// 
/// The explicit padding ensures deterministic memory layout, preventing subtle bugs
/// when the same data is accessed from different program versions or architectures.
#[derive(Clone, Copy, Debug, Pod, Zeroable, InitSpace)]
#[repr(C)]
pub struct PriceData {
    /// Price value in base units (scaled by 10^expo).
    /// Signed to support negative values for spreads and derivatives.
    pub price: i64,
    
    /// Confidence interval representing price uncertainty.
    /// Higher values indicate less reliable price data.
    pub conf: u64,
    
    /// Unix timestamp when this price was last updated.
    /// Used for staleness detection and TWAP calculations.
    pub timestamp: u64,
    
    /// Base-10 exponent for price scaling (e.g., -6 for micro-units).
    /// Enables representation of assets with vastly different nominal values.
    pub expo: i32,
    
    /// Explicit padding for deterministic struct alignment.
    /// Prevents architecture-dependent layout variations.
    pub _padding: [u8; 4],
}

impl OracleState {

    /// Updates the number of active price feeds with bounds checking.
    /// 
    /// # Safety Considerations
    /// 
    /// This method enforces critical invariants:
    /// - Prevents buffer overruns when iterating over active feeds
    /// - Ensures active_feeds() slice operations remain memory-safe
    /// - Validates against compile-time constant to catch configuration errors
    /// 
    /// The explicit bounds check is essential because active_feed_count controls
    /// slice operations throughout the codebase. An invalid count could lead to
    /// reading uninitialized memory or accessing out-of-bounds array elements.
    pub fn set_active_feed_count(&mut self, count: u8) -> Result<()> {
        require!(
            (count as usize) <= MAX_PRICE_FEEDS,
            StateError::TooManyActiveFeeds
        );

        self.active_feed_count = count;
        
        Ok(())
    }

    /// Delegation to flag-specific circuit breaker check.
    /// 
    /// This convenience method maintains API consistency while enabling the compiler
    /// to inline the flag check, eliminating any performance overhead from the delegation.
    #[inline(always)]
    pub fn is_circuit_breaker_enabled(&self) -> bool {
        self.flags.is_circuit_breaker_enabled()
    }

    /// Returns slice view of currently active price feeds.
    /// 
    /// # Performance Rationale
    /// 
    /// This method avoids heap allocation by returning a slice view instead of Vec.
    /// The slice is guaranteed safe because set_active_feed_count() validates the
    /// count bounds. This pattern enables zero-cost iteration over active feeds
    /// while maintaining memory safety.
    /// 
    /// The inline annotation ensures this becomes a no-op bounds check + pointer arithmetic
    /// in optimized builds, with no function call overhead.
    #[inline(always)]
    pub fn active_feeds(&self) -> &[PriceFeed] {
        &self.price_feeds[..self.active_feed_count as usize]
    }

    /// Validates all active feeds against manipulation detection criteria.
    /// 
    /// # Anti-MEV Design
    /// 
    /// This method implements multiple layers of manipulation resistance:
    /// 
    /// 1. **LP Concentration Limits**: Prevents single liquidity provider from
    ///    controlling price discovery on individual feeds
    /// 2. **Manipulation Scoring**: Detects coordinated attacks across feeds
    ///    using statistical analysis of price movements and volume patterns
    /// 3. **Active Feed Filtering**: Ignores disabled feeds to prevent attackers
    ///    from gaming the system by temporarily disabling honest sources
    /// 
    /// The early continue pattern optimizes for the common case where most feeds
    /// are active, minimizing branch mispredictions in the hot path.
    pub fn check_manipulation_resistance(&self) -> Result<()> {
        for feed in self.active_feeds() {
            // Skip manipulation checks for inactive feeds to prevent
            // attackers from gaming the system by disabling honest sources
            if !feed.flags.is_active(){
                continue;
            }

            // Prevent single LP from controlling price discovery
            if feed.lp_concentration > MAX_LP_CONCENTRATION {
                return Err(StateError::ExcessiveLpConcentration.into());
            }

            // Detect coordinated manipulation across multiple vectors
            if feed.manipulation_score > self.manipulation_threshold {
                return Err(StateError::ManipulationDetected.into());
            }
        }

        Ok(())
    }
}