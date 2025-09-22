use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

/// Individual price feed data source with quality metrics and manipulation detection.
/// 
/// # Architecture Philosophy
/// 
/// This struct represents a single price data source within the multi-tier oracle system.
/// The design prioritizes several key concerns:
/// 
/// - **Zero-Copy Performance**: Pod + Zeroable traits enable direct memory access without
///   serialization overhead, critical for high-frequency price updates on Solana
/// - **Manipulation Resistance**: Multiple quality metrics (LP concentration, volume,
///   liquidity depth) enable sophisticated manipulation detection algorithms
/// - **Weighted Aggregation**: Weight field allows dynamic rebalancing based on source
///   reliability and market conditions
/// - **Memory Efficiency**: Compact layout with carefully sized fields minimizes account
///   storage costs while maintaining sufficient precision
/// 
/// The struct layout orders fields by access frequency to optimize cache performance
/// during price aggregation operations, which are the most compute-intensive workload.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Pod, Zeroable, InitSpace, Default)]
#[repr(C)]
pub struct PriceFeed {
    /// Unique identifier for the price data source.
    /// Used for source validation and preventing duplicate feed registration.
    pub source_address: Pubkey,
    
    /// Most recent price value from this source in scaled integer format.
    /// Signed to support negative prices for derivatives and spread instruments.
    pub last_price: i128,

    /// 24-hour trading volume to assess market activity and manipulation resistance.
    /// Higher volume typically correlates with harder-to-manipulate prices.
    pub volume_24h: i128,
    
    /// Available liquidity depth for price impact analysis.
    /// Used to estimate how much capital would be needed to move the price significantly.
    pub liquidity_depth: i128,
    
    /// Confidence interval for the last price reading.
    /// Higher values indicate less reliable data, used in weighted aggregation.
    pub last_conf: u64,
    
    /// Unix timestamp of the most recent price update.
    /// Critical for staleness detection and temporal weighting in TWAP calculations.
    pub last_update: i64,
    
    /// Decimal exponent for price scaling (e.g., -6 for microunits).
    /// Enables consistent representation across assets with vastly different nominal values.
    pub last_expo: i32,
    
    /// Relative importance weight in aggregation calculations (basis points).
    /// Dynamically adjusted based on source reliability, volume, and market conditions.
    pub weight: u16,
    
    /// Percentage of liquidity controlled by largest provider (basis points).
    /// High concentration indicates vulnerability to single-actor manipulation.
    pub lp_concentration: u16,
    
    /// Computed manipulation risk score based on price movement patterns.
    /// Incorporates statistical analysis of price vs volume relationships.
    pub manipulation_score: u16,
    
    /// Type of price source for risk assessment and aggregation strategy.
    /// Different source types have different trust profiles and manipulation vectors.
    pub source_type: u8,
    
    /// Bitfield for feed operational state and quality indicators.
    /// Compact representation enables efficient bulk operations on feed status.
    pub flags: FeedFlags,
    
    /// Explicit padding ensures deterministic struct layout across platforms.
    /// Prevents subtle bugs from compiler-dependent field alignment decisions.
    pub _padding: [u8; 4],
}

impl PriceFeed {
    /// Gets the source type with type safety and error handling.
    /// Protects against corrupted account data by providing graceful fallback.
    #[inline(always)]
    pub fn get_source_type(self) -> SourceType {
        SourceType::from_u8_or_default(self.source_type)
    }

    /// Sets the source type with automatic u8 conversion.
    /// Maintains type safety while preserving zero-copy storage benefits.
    #[inline(always)]
    pub fn set_source_type(&mut self, source_type: SourceType) {
        self.source_type = source_type.as_u8();
    }

    /// Checks if the feed matches a specific source type.
    /// Optimized for filtering operations during aggregation.
    #[inline(always)]
    pub fn is_source_type(self, source_type: SourceType) -> bool {
        self.source_type == source_type.as_u8()
    }
}

/// Compact bitfield for price feed status and quality indicators.
/// 
/// # Design Rationale
/// 
/// Uses u8 instead of u32 to minimize memory footprint within the larger PriceFeed struct.
/// Since feed flags are checked frequently during aggregation, the smaller size improves
/// cache efficiency. The transparent wrapper provides type safety while maintaining
/// zero-cost abstractions.
/// 
/// The flag-based approach enables:
/// - **Atomic State Changes**: Multiple feed properties can be updated simultaneously
/// - **Efficient Bulk Operations**: Bitwise operations on multiple feeds in parallel
/// - **Extensibility**: New quality indicators can be added without schema migration
/// - **Performance**: Single instruction flag checks vs multiple field comparisons
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Default, InitSpace)]
#[repr(transparent)]
pub struct FeedFlags(u8);

impl FeedFlags {
    /// Flag definitions with explicit binary values for auditability.
    /// Each flag represents a distinct aspect of feed quality or operational state.
    
    /// Feed is currently operational and providing price updates.
    /// Disabled feeds are excluded from aggregation to prevent stale data influence.
    pub const ACTIVE: Self                = Self(0b0000_0001);
    
    /// Source has established reliability through historical performance.
    /// Trusted feeds receive higher weights in emergency fallback scenarios.
    pub const TRUSTED: Self               = Self(0b0000_0010);
    
    /// Price data is outdated beyond acceptable thresholds.
    /// Triggers automatic feed exclusion to prevent stale price contamination.
    pub const STALE: Self                 = Self(0b0000_0100);
    
    /// Algorithmic detection of potential price manipulation.
    /// Causes immediate feed quarantine pending manual review.
    pub const MANIPULATION_DETECTED: Self = Self(0b0000_1000);

    /// Bitmask for all currently defined flags.
    /// Enables forward-compatible deserialization that gracefully handles unknown flags.
    pub const VALID_MASK: u8 = Self::ACTIVE.0
        | Self::TRUSTED.0
        | Self::STALE.0
        | Self::MANIPULATION_DETECTED.0;

    /// Creates empty flag set with all indicators disabled.
    /// const fn allows compile-time initialization for default instances.
    #[inline(always)] 
    pub const fn new() -> Self { Self(0) }

    /// Tests whether any bits from the specified flag pattern are set.
    /// Single-instruction bitwise AND operation for maximum performance.
    #[inline(always)] 
    pub fn has(self, flag: Self) -> bool { 
        (self.0 & flag.0) != 0 
    }

    /// Enables the specified flag bits using bitwise OR.
    /// Preserves existing flag state while activating new indicators.
    #[inline(always)] 
    pub fn set(&mut self, flag: Self) {
        self.0 |= flag.0; 
    }

    /// Disables the specified flag bits using bitwise AND with complement.
    /// Preserves other flags while clearing target indicators.
    #[inline(always)] 
    pub fn clear(&mut self, flag: Self) { 
        self.0 &= !flag.0; 
    }

    /// Inverts the specified flag bits using bitwise XOR.
    /// Useful for state machines where flag transitions depend on current state.
    #[inline(always)] 
    pub fn toggle(&mut self, flag: Self)  { 
        self.0 ^= flag.0; 
    }

    /// Conditionally sets or clears flag based on boolean condition.
    /// Eliminates branching in calling code for cleaner conditional flag management.
    #[inline(always)] 
    pub fn set_to(&mut self, flag: Self, on: bool) {
        if on { self.set(flag) } else { self.clear(flag) }
    }

    /// High-level semantic accessors for common feed quality checks.
    /// These compile to identical assembly as direct flag operations but provide
    /// better API ergonomics and self-documenting code.
    
    #[inline(always)] 
    pub fn is_active(self) -> bool { 
        self.has(Self::ACTIVE) 
    }

    #[inline(always)] 
    pub fn is_trusted(self) -> bool { 
        self.has(Self::TRUSTED) 
    }

    #[inline(always)] 
    pub fn is_stale(self) -> bool { 
        self.has(Self::STALE) 
    }

    #[inline(always)] 
    pub fn is_manipulation_detected(self) -> bool { 
        self.has(Self::MANIPULATION_DETECTED) 
    }

    /// Serialization utilities for account data persistence.
    
    /// Extracts raw u8 value for storage in account data.
    /// const fn enables compile-time evaluation where applicable.
    #[inline(always)] 
    pub const fn as_u8(self) -> u8 { 
        self.0 
    }

    /// Creates FeedFlags from raw u8, filtering unknown flag bits.
    /// Defensive deserialization prevents crashes when reading data written by
    /// newer program versions that define additional flags.
    #[inline(always)] 
    pub const fn from_u8_truncate(value: u8) -> Self {
        Self(value & Self::VALID_MASK)
    }
}

/// Classification of price data sources for risk assessment and aggregation strategy.
/// 
/// # Design Philosophy
/// 
/// Different source types have fundamentally different trust models and manipulation
/// attack vectors. This enum enables the oracle to apply appropriate validation logic
/// and weighting strategies for each source category:
/// 
/// - **DEX sources** are vulnerable to flash loan attacks but provide real market depth
/// - **CEX sources** offer high volume but introduce counterparty and API risks  
/// - **Oracle sources** provide filtered data but may have single points of failure
/// - **Aggregator sources** reduce variance but can amplify systemic biases
/// 
/// The u8 representation minimizes storage overhead while providing sufficient
/// extensibility for future source types. Explicit discriminant values ensure
/// stable serialization across program versions.
#[derive(Clone, Copy, Debug, PartialEq, AnchorSerialize, AnchorDeserialize)]
#[repr(u8)]
pub enum SourceType {
    /// Decentralized exchange (e.g., Uniswap, Serum)
    /// High manipulation risk but reflects actual trading activity
    DEX = 0,
    
    /// Centralized exchange (e.g., Binance, Coinbase)  
    /// Lower manipulation risk but introduces counterparty dependencies
    CEX = 1,
    
    /// External oracle service (e.g., Chainlink, Pyth)
    /// Pre-filtered data but potential single points of failure
    Oracle = 2,
    
    /// Price aggregator service combining multiple sources
    /// Reduced variance but may amplify correlated errors
    Aggregator = 3,
}

impl SourceType {
    /// Converts SourceType to its underlying u8 discriminant for zero-copy storage.
    /// The direct enum-to-integer cast leverages Rust's guaranteed enum representation
    /// to provide zero-cost conversion while maintaining type safety at the API boundary.
    #[inline(always)]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Attempts to reconstruct SourceType from raw u8 with validation.
    /// 
    /// # Error Handling Philosophy
    /// 
    /// Returns `Option<Self>` rather than panicking to enable graceful handling of
    /// corrupted or future-versioned account data. This defensive approach prevents
    /// program crashes when encountering unknown source type discriminants that might
    /// be introduced in future program versions.
    #[inline(always)]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::DEX),
            1 => Some(Self::CEX),
            2 => Some(Self::Oracle),
            3 => Some(Self::Aggregator),
            // Explicit rejection of unknown values enables graceful degradation
            // and prevents silent corruption from unrecognized discriminants
            _ => None,
        }
    }

    /// Reconstructs SourceType from u8 with conservative fallback for resilience.
    /// 
    /// # Defensive Programming Strategy
    /// 
    /// This method prioritizes system stability over strict validation by providing
    /// a sensible default when encountering unknown source type values. The choice
    /// of `DEX` as the fallback is intentionally conservative:
    /// 
    /// - **Maximum Scrutiny**: DEX sources receive the highest manipulation detection
    ///   thresholds, ensuring unknown sources are treated with appropriate suspicion
    /// - **Fail-Safe Behavior**: If account data is corrupted, defaulting to the most
    ///   restrictive source type prevents potential security vulnerabilities
    /// - **Operational Continuity**: Enables the oracle to continue functioning even
    ///   when encountering data from newer program versions with extended enums
    #[inline(always)]
    pub const fn from_u8_or_default(value: u8) -> Self {
        match Self::from_u8(value) {
            Some(source_type) => source_type,
            // Conservative fallback: DEX requires highest manipulation detection thresholds,
            // ensuring unknown source types are treated with maximum suspicion
            None => Self::DEX,
        }
    }
}
