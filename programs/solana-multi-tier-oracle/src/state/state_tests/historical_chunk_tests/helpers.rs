use crate::state::historical_chunk::{HistoricalChunk, PricePoint};
use crate::state::oracle_state::{OracleState, PriceData, StateFlags, Version};
use crate::state::price_feed::PriceFeed;
use crate::utils::constants::{
    BUFFER_SIZE, MAX_HISTORICAL_CHUNKS, MAX_PRICE_FEEDS, MIN_HISTORICAL_INTERVAL,
};
use anchor_lang::prelude::Pubkey;
use proptest::arbitrary::any;
use proptest::prelude::*;
use std::mem::{size_of, MaybeUninit};
use std::ptr;

/// Convenience constant for working with `u16` indices inside the circular buffer.
pub(crate) const BUFFER_SIZE_U16: u16 = BUFFER_SIZE as u16;

macro_rules! assert_chunk_invariants {
    ($chunk:expr) => {{
        let chunk_ref = &$chunk;
        assert!(
            chunk_ref.head < BUFFER_SIZE_U16,
            "head pointer must stay within circular bounds"
        );
        assert!(
            chunk_ref.tail < BUFFER_SIZE_U16,
            "tail pointer must stay within circular bounds"
        );
        assert!(
            chunk_ref.count <= BUFFER_SIZE_U16,
            "count cannot exceed fixed buffer capacity"
        );

        if chunk_ref.head == chunk_ref.tail {
            assert!(
                chunk_ref.count == 0 || chunk_ref.count == BUFFER_SIZE_U16,
                "head == tail is only legal when buffer is empty or completely full"
            );
        }
    }};
}

pub(crate) use assert_chunk_invariants;

/// Creates a zeroed-out `HistoricalChunk` fixture with deterministic defaults.
///
/// Keeping the constructor explicit makes the tests resilient to future schema
/// tweaks — if a new field is added, the compiler forces us to initialize it here.
pub(crate) fn empty_chunk() -> HistoricalChunk {
    HistoricalChunk {
        chunk_id: 0,
        head: 0,
        tail: 0,
        count: 0,
        creation_timestamp: 0,
        next_chunk: Pubkey::default(),
        oracle_state: Pubkey::default(),
        price_points: [PricePoint::default(); BUFFER_SIZE],
        bump: 0,
        reserved: [0; 511],
    }
}

/// Produces a deterministic `PricePoint` whose values are spaced far enough
/// apart to catch accidental field swaps during assertions.
pub(crate) fn deterministic_price_point(seed: i64) -> PricePoint {
    PricePoint {
        price: 1_000_000_000_000 + (seed as i128 * 997),
        volume: 500_000_000_000 + (seed as i128 * 4096),
        conf: (seed.unsigned_abs() % 50_000) + 42,
        timestamp: 1_700_000_000 + seed * MIN_HISTORICAL_INTERVAL,
    }
}

/// Extremal price point generator used in stress tests to flush out overflow
/// behaviour when alternating between signed bounds.
pub(crate) fn alternating_extreme_point(index: usize) -> PricePoint {
    if index % 2 == 0 {
        PricePoint {
            price: i128::MAX,
            volume: 0,
            conf: u64::MAX,
            timestamp: 1_700_000_000 + (index as i64 * MIN_HISTORICAL_INTERVAL),
        }
    } else {
        PricePoint {
            price: i128::MIN + 1, // avoid MIN_NEGATE overflow in downstream maths
            volume: i128::MIN + 1,
            conf: 1,
            timestamp: 1_700_000_000 + (index as i64 * MIN_HISTORICAL_INTERVAL),
        }
    }
}

/// Helper for comparing `PricePoint` values field-by-field with descriptive
/// panic messages — derived equality is not available because the struct omits
/// `PartialEq` for zero-copy ergonomics.
pub(crate) fn assert_price_point_eq(actual: &PricePoint, expected: &PricePoint) {
    assert_eq!(actual.price, expected.price, "price mismatch");
    assert_eq!(actual.volume, expected.volume, "volume mismatch");
    assert_eq!(actual.conf, expected.conf, "confidence mismatch");
    assert_eq!(actual.timestamp, expected.timestamp, "timestamp mismatch");
}

/// Collects the logical FIFO ordering from a circular buffer by iterating from
/// `tail` across `count` entries. This is useful for asserting historical order
/// in tests that operate on saturated buffers.
pub(crate) fn collect_fifo_view(chunk: &HistoricalChunk) -> Vec<PricePoint> {
    let mut out = Vec::with_capacity(chunk.count as usize);
    let mut idx = chunk.tail;
    for _ in 0..chunk.count {
        out.push(chunk.price_points[idx as usize]);
        idx = (idx + 1) & (BUFFER_SIZE_U16 - 1);
    }
    out
}

/// Copies the raw bytes underpinning a historical chunk. This mirrors Anchor's
/// zero-copy account loading behaviour and is used to validate deterministic
/// byte representations without relying on additional trait implementations.
pub(crate) fn chunk_to_bytes(chunk: &HistoricalChunk) -> Vec<u8> {
    let mut bytes = vec![0u8; size_of::<HistoricalChunk>()];
    unsafe {
        ptr::copy_nonoverlapping(
            (chunk as *const HistoricalChunk) as *const u8,
            bytes.as_mut_ptr(),
            bytes.len(),
        );
    }
    bytes
}

/// Reconstructs a `HistoricalChunk` from a byte slice previously produced by
/// `chunk_to_bytes`. This simulates the zero-copy account deserialization path
/// used inside the Solana runtime.
pub(crate) fn chunk_from_bytes(bytes: &[u8]) -> HistoricalChunk {
    assert_eq!(bytes.len(), size_of::<HistoricalChunk>());
    let mut uninit = MaybeUninit::<HistoricalChunk>::uninit();
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), uninit.as_mut_ptr() as *mut u8, bytes.len());
        uninit.assume_init()
    }
}

/// Prepares an `OracleState` instance with benign defaults so tests can exercise
/// cross-structure interactions without re-declaring boilerplate for every field.
pub(crate) fn minimal_oracle_state() -> OracleState {
    OracleState {
        authority: Pubkey::new_unique(),
        version: Version {
            major: 1,
            minor: 0,
            patch: 0,
            _padding: 0,
        },
        flags: StateFlags::default(),
        last_update: 0,
        current_price: PriceData::default(),
        price_feeds: [PriceFeed::default(); MAX_PRICE_FEEDS],
        twap_window: 0,
        current_chunk_index: 0,
        max_chunk_size: BUFFER_SIZE_U16,
        confidence_threshold: 0,
        manipulation_threshold: 0,
        active_feed_count: 0,
        bump: 0,
        governance_bump: 0,
        historical_chunks: [Pubkey::default(); MAX_HISTORICAL_CHUNKS],
        emergency_admin: Pubkey::default(),
        asset_seed: [0; 32],
        reserved: [0; 513],
    }
}

/// Strategy that emits arbitrary `PricePoint` values while ensuring timestamps
/// stay within a realistic (but broad) range. The wide domain helps fuzz tests
/// poke at bitmask wraparound and signed arithmetic simultaneously.
#[allow(dead_code)]
pub(crate) fn proptest_price_point_strategy() -> impl Strategy<Value = PricePoint> {
    (
        any::<i128>(),
        any::<i128>(),
        any::<u64>(),
        (-1_900_000_000_i64..=1_900_000_000_i64),
    )
        .prop_map(|(price, volume, conf, timestamp)| PricePoint {
            price,
            volume,
            conf,
            timestamp,
        })
}
