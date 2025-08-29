use crate::utils::constants::BUFFER_SIZE;
use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

/// High-performance circular buffer for historical price data storage.
/// 
/// # Architecture Design Rationale
/// 
/// This struct implements a circular buffer using fixed-size arrays to provide O(1) insertion
/// and retrieval of historical price data while working within Solana's account size constraints.
/// Key design decisions:
/// 
/// - **Zero-Copy Performance**: Direct memory access eliminates serialization overhead during
///   frequent price updates, critical for maintaining low latency in high-frequency scenarios
/// - **Circular Buffer Pattern**: Prevents expensive array shifts when the buffer is full,
///   maintaining constant-time insertion regardless of historical depth
/// - **Fixed Account Size**: Predictable storage costs enable accurate rent exemption calculations
///   and prevent account size growth from causing transaction failures
/// - **Linked List Architecture**: Chain of chunks enables unbounded historical storage while
///   respecting individual account size limits (≤10MB on Solana)
/// 
/// # Memory Layout Optimization
/// 
/// Fields are ordered to minimize memory overhead and optimize cache performance:
/// - Metadata fields grouped together for sequential access during buffer management
/// - Large price_points array placed after metadata to avoid fragmentation
/// - Buffer size chosen as power-of-2 to enable efficient modulo operations via bitmasking
/// 
/// # Trade-offs
/// 
/// - **Space vs Time**: Fixed allocation wastes space when partially filled but eliminates
///   reallocation overhead and provides predictable performance characteristics
/// - **Complexity vs Performance**: Circular buffer logic is more complex than simple append
///   but provides superior performance for the historical data access patterns
#[account(zero_copy)]
#[derive(InitSpace)]
#[repr(C)]
pub struct HistoricalChunk {
    /// Unique identifier for this chunk within the historical chain.
    /// Enables efficient lookups and debugging of chunk ordering issues.
    pub chunk_id: u16,
    
    /// Index where the next price point will be written.
    /// Forms the "write pointer" in the circular buffer implementation.
    pub head: u16,
    
    /// Index of the oldest valid price point when buffer is full.
    /// Forms the "read pointer" for maintaining FIFO ordering in circular buffer.
    pub tail: u16,
    
    /// Current number of valid price points stored (0 to BUFFER_SIZE).
    /// Distinguishes between empty, partially filled, and full buffer states.
    pub count: u16,
    
    /// Unix timestamp when this chunk was first created.
    /// Used for chunk lifecycle management and debugging temporal ordering.
    pub creation_timestamp: u64,
    
    /// Reference to the subsequent chunk in the historical chain.
    /// Enables traversal of unbounded historical data across multiple accounts.
    pub next_chunk: Pubkey,
    
    /// Fixed-size circular buffer storing price data points.
    /// Size chosen as power-of-2 to enable efficient modulo via bitwise AND operations.
    pub price_points: [PricePoint; BUFFER_SIZE],
    
    /// Reserved space for future schema evolution without breaking changes.
    /// Prevents need for complex data migration when adding new functionality.
    pub reserved: [u64; 8],
}

/// Individual price data point optimized for historical storage and analysis.
/// 
/// # Design Philosophy
/// 
/// This struct balances storage efficiency with analytical utility for historical price data.
/// Unlike real-time price feeds that prioritize update frequency, historical data emphasizes:
/// 
/// - **Analytical Completeness**: Volume data enables sophisticated manipulation detection
///   and market microstructure analysis over time
/// - **Storage Efficiency**: Compact representation minimizes account storage costs for
///   large historical datasets spanning months or years
/// - **Cross-Platform Compatibility**: Explicit padding ensures consistent layout across
///   different architectures and compiler versions
/// 
/// # Field Selection Rationale
/// 
/// Each field serves specific analytical or operational purposes:
/// - Price + confidence + expo: Core price representation matching industry standards
/// - Timestamp: Enables temporal analysis and time-weighted calculations
/// - Volume: Critical for detecting manipulation patterns and market health metrics
/// - Padding: Prevents subtle alignment bugs that could corrupt historical data
#[derive(Clone, Copy, Debug, Pod, Zeroable, InitSpace)]
#[repr(C)]
pub struct PricePoint {
    /// Price value in scaled integer format (apply expo for decimal representation).
    /// Signed to accommodate negative values for derivatives and spread instruments.
    pub price: i64,
    
    /// Confidence interval indicating price uncertainty at time of recording.
    /// Higher values suggest less reliable data, useful for historical quality analysis.
    pub conf: u64,
    
    /// Unix timestamp when this price point was recorded.
    /// Essential for temporal analysis and time-weighted average calculations.
    pub timestamp: u64,
    
    /// Trading volume associated with this price point.
    /// Enables sophisticated manipulation detection and market depth analysis over time.
    pub volume: u64,
    
    /// Base-10 exponent for price scaling (e.g., -6 for microunits).
    /// Maintains consistency with real-time price representation standards.
    pub expo: i32,
    
    /// Explicit padding ensures deterministic memory layout.
    /// Critical for historical data integrity across different deployment environments.
    pub _padding: [u8; 4],
}

impl HistoricalChunk {
    /// Tests whether this chunk links to a subsequent chunk in the historical chain.
    /// 
    /// This method enables efficient traversal of historical data across multiple accounts
    /// without requiring expensive Pubkey comparisons in tight loops. The inline annotation
    /// ensures this becomes a simple pointer comparison in optimized builds.
    #[inline(always)]
    pub fn has_next(&self) -> bool {
        self.next_chunk != Pubkey::default()
    }
    
    /// Inserts a new price point using circular buffer semantics for O(1) performance.
    /// 
    /// # Algorithm Design
    /// 
    /// This implementation prioritizes constant-time insertion over memory efficiency:
    /// 
    /// 1. **Direct Assignment**: Overwrites the slot at head position without shifting elements
    /// 2. **Bitwise Modulo**: Uses `& (BUFFER_SIZE - 1)` instead of `% BUFFER_SIZE` for
    ///    faster wraparound calculation (requires BUFFER_SIZE to be power-of-2)
    /// 3. **Conditional Tail Update**: Only advances tail when buffer is full, maintaining
    ///    FIFO ordering while preserving all data during initial fill phase
    /// 
    /// # Performance Characteristics
    /// 
    /// - Time Complexity: O(1) regardless of buffer state or historical depth
    /// - Space Complexity: O(1) with no dynamic allocation
    /// - Cache Efficiency: Sequential head advancement optimizes memory access patterns
    /// 
    /// # Safety Considerations
    /// 
    /// The bitwise AND operation for wraparound is only correct when BUFFER_SIZE is a
    /// power of 2. This constraint is enforced at compile time by the constants module.
    pub fn push(&mut self, point: PricePoint) {
        // Overwrite the slot at head position - no need to shift existing elements
        self.price_points[self.head as usize] = point;
        
        // Advance head with efficient bitwise wraparound (requires power-of-2 buffer size)
        self.head = (self.head + 1) & (BUFFER_SIZE as u16 - 1);

        if self.count < BUFFER_SIZE as u16 {
            // Buffer not yet full - just increment count to track valid elements
            self.count += 1;
        } else {
            // Buffer full - advance tail to maintain FIFO ordering and fixed capacity
            self.tail = (self.tail + 1) & (BUFFER_SIZE as u16 - 1);
        }
    }

    /// Retrieves the most recently inserted price point with zero-copy semantics.
    /// 
    /// # Return Value Strategy
    /// 
    /// Returns `Option<&PricePoint>` rather than copying the data to avoid allocation
    /// overhead. This enables efficient chaining of operations on the latest price
    /// without heap pressure.
    /// 
    /// # Index Calculation Rationale
    /// 
    /// The latest element is always at `(head - 1) % BUFFER_SIZE`, but this requires
    /// careful handling of the wraparound case when head = 0. The explicit conditional
    /// is more readable and equally efficient after compiler optimization compared to
    /// modular arithmetic approaches.
    pub fn latest(&self) -> Option<&PricePoint> {
        if self.count == 0 {
            None
        } else {
            // Calculate index of most recently inserted element
            // Handle wraparound case explicitly for clarity
            let latest_index = if self.head == 0 {
                BUFFER_SIZE - 1  // Wrapped around, latest is at end of buffer
            } else {
                (self.head - 1) as usize  // Latest is immediately before head
            };
            Some(&self.price_points[latest_index])
        }
    }
}