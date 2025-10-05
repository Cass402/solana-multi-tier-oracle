//! Tests that exercise serialization round-trips and the integration hook into
//! `OracleState::check_snapshot_requirements_from_history`.
//!
//! Intent and risk model:
//! - These tests validate two orthogonal but related contracts: (1) that the
//!   in-memory representation of historical chunks serializes to deterministic
//!   byte images compatible with zero-copy readers, and (2) that the snapshot
//!   sufficiency checks (time-window and count) behave as the governance
//!   policy intends. Both areas are sensitive: the former to layout/ABI
//!   drift, the latter to logic/regression errors that could allow stale or
//!   insufficient historical data to be accepted.

use super::helpers::{
    assert_chunk_invariants, assert_price_point_eq, chunk_from_bytes, chunk_to_bytes,
    collect_fifo_view, deterministic_price_point, empty_chunk, minimal_oracle_state,
    BUFFER_SIZE_U16,
};
use crate::state::historical_chunk::HistoricalChunk;
use crate::state::oracle_state::OracleState;
use crate::state::snapshot_status::SnapshotStatus;
use crate::utils::constants::{BUFFER_SIZE, MIN_HISTORICAL_INTERVAL, SECONDS_PER_HOUR};
use anchor_lang::prelude::Pubkey;

fn build_chunk_with_seed_range(
    start_seed: i64,
    count: usize,
    oracle_state: Pubkey,
    chunk_id: u16,
) -> HistoricalChunk {
    let mut chunk = empty_chunk();
    chunk.chunk_id = chunk_id;
    chunk.oracle_state = oracle_state;

    if count > 0 {
        chunk.creation_timestamp = deterministic_price_point(start_seed).timestamp;
    }

    let mut seed = start_seed;
    for _ in 0..count {
        chunk.push(deterministic_price_point(seed));
        seed += 1;
    }

    chunk
}

#[test]
fn linked_chunks_preserve_fifo_across_chain() {
    // This test models a multi-chunk chain of historical data as used when
    // the circular buffer rolls over into a new account. It asserts the
    // higher-level invariant that traversing successive chunks yields a
    // single, monotonically increasing FIFO of price points. This is crucial
    // because snapshot and TWAP consumers stitch history across chunk
    // boundaries and must not see gaps or re-ordering.
    struct ChunkAccount {
        address: Pubkey,
        data: HistoricalChunk,
    }

    let oracle_state_pk = Pubkey::new_unique();

    // First chunk saturates to full capacity before rollover.
    let mut chunk_one = ChunkAccount {
        address: Pubkey::new_unique(),
        data: empty_chunk(),
    };
    chunk_one.data.chunk_id = 0;
    chunk_one.data.oracle_state = oracle_state_pk;
    chunk_one.data.creation_timestamp = deterministic_price_point(0).timestamp;

    for seed in 0..(BUFFER_SIZE as i64) {
        chunk_one.data.push(deterministic_price_point(seed));
    }
    assert_chunk_invariants!(chunk_one.data);
    assert_eq!(chunk_one.data.count, BUFFER_SIZE_U16);
    let chunk_one_latest = chunk_one
        .data
        .latest()
        .expect("full chunk should yield latest element");
    assert_price_point_eq(
        &deterministic_price_point(BUFFER_SIZE as i64 - 1),
        chunk_one_latest,
    );

    // Second chunk picks up subsequent history once the chain rotates.
    let rollover_len = BUFFER_SIZE / 4; // partial fill to ensure head/tail diverge
    let mut chunk_two = ChunkAccount {
        address: Pubkey::new_unique(),
        data: empty_chunk(),
    };
    chunk_two.data.chunk_id = 1;
    chunk_two.data.oracle_state = oracle_state_pk;
    chunk_two.data.creation_timestamp = deterministic_price_point(BUFFER_SIZE as i64).timestamp;

    for offset in 0..(rollover_len as i64) {
        let seed = BUFFER_SIZE as i64 + offset;
        chunk_two.data.push(deterministic_price_point(seed));
    }
    assert_chunk_invariants!(chunk_two.data);
    assert!(chunk_two.data.count < BUFFER_SIZE_U16);

    // Link the chain and verify navigation semantics.
    chunk_one.data.next_chunk = chunk_two.address;
    assert!(
        chunk_one.data.has_next(),
        "first chunk should report successor"
    );
    assert!(
        !chunk_two.data.has_next(),
        "terminal chunk must not advertise a successor"
    );
    assert_eq!(chunk_one.data.oracle_state, chunk_two.data.oracle_state);

    // Reconstruct combined FIFO history by traversing both chunks in order.
    let mut consolidated_history = collect_fifo_view(&chunk_one.data);
    consolidated_history.extend(collect_fifo_view(&chunk_two.data));

    let expected_len = BUFFER_SIZE + rollover_len;
    assert_eq!(consolidated_history.len(), expected_len);

    for (idx, actual) in consolidated_history.iter().enumerate() {
        let expected_point = deterministic_price_point(idx as i64);
        assert_price_point_eq(actual, &expected_point);
    }

    // Latest element across the chain should come from the second chunk.
    let chain_latest = chunk_two
        .data
        .latest()
        .expect("second chunk must contain recent entries");
    let expected_latest = deterministic_price_point(BUFFER_SIZE as i64 + rollover_len as i64 - 1);
    assert_price_point_eq(chain_latest, &expected_latest);
}
/// Roundtrip through the zero-copy byte image to prove the struct retains
/// deterministic representations compatible with Anchor account loading.
///
/// Why this matters:
/// - Zero-copy readers map account bytes directly into structs. Any
///   divergence between the in-memory layout and its raw byte image will
///   break that mapping. We assert field-level equality and normalized
///   reserved/padding to catch issues that could emerge after refactors or
///   compiler toolchain changes.
#[test]
fn anchor_roundtrip_preserves_historical_chunk_bytes() {
    let oracle_state_pk = Pubkey::new_unique();
    let mut chunk = build_chunk_with_seed_range(-42, BUFFER_SIZE / 2, oracle_state_pk, 7);
    chunk.next_chunk = Pubkey::new_unique();
    chunk.bump = 3;

    let buffer = chunk_to_bytes(&chunk);
    let reread = chunk_from_bytes(&buffer);

    assert_eq!(reread.chunk_id, chunk.chunk_id);
    assert_eq!(reread.creation_timestamp, chunk.creation_timestamp);
    assert_eq!(reread.count, chunk.count);
    assert_eq!(reread.head, chunk.head);
    assert_eq!(reread.tail, chunk.tail);
    assert_eq!(reread.bump, chunk.bump);
    assert_eq!(reread.next_chunk, chunk.next_chunk);
    assert_eq!(reread.oracle_state, chunk.oracle_state);
    assert!(reread.reserved.iter().all(|byte| *byte == 0));
    assert_price_point_eq(&reread.price_points[0], &chunk.price_points[0]);
}

fn build_historical_span(total_points: usize, end_seed: i64) -> Vec<HistoricalChunk> {
    let oracle_state_pk = Pubkey::new_unique();
    let mut remaining = total_points;
    let mut seed = end_seed - total_points as i64 + 1;
    let mut chunks = Vec::new();

    let mut chunk_id = 0u16;
    while remaining > 0 {
        let take = remaining.min(BUFFER_SIZE);
        let chunk = build_chunk_with_seed_range(seed, take, oracle_state_pk, chunk_id);
        chunks.push(chunk);
        remaining -= take;
        seed += take as i64;
        chunk_id += 1;
    }

    chunks
}

/// Integration test proving that real historical chunks can satisfy a 72-hour
/// redemption window, demonstrating the intended flow for production snapshot
/// validation.
///
/// Design rationale:
/// - Snapshot sufficiency is a safety-critical check: accepting an
///   insufficient snapshot may expose downstream systems (liquidation,
///   settlement) to stale data. We construct contiguous history to show the
///   ideal path meets governance requirements.
#[test]
fn seventy_two_hour_requirement_succeeds_with_contiguous_history() {
    let oracle_state: OracleState = minimal_oracle_state();
    let total_points = 289; // 72 hours at 15-minute cadence requires 289 snapshots for >= 72h span
    let current_seed = 0i64;
    let chunks = build_historical_span(total_points, current_seed);
    let current_timestamp = super::helpers::deterministic_price_point(current_seed).timestamp;

    let status =
        oracle_state.check_snapshot_requirements_from_history(&chunks, current_timestamp, 72);
    match status {
        SnapshotStatus::Sufficient {
            snapshot_count,
            time_span_hours,
            ..
        } => {
            assert_eq!(
                snapshot_count, total_points as u16,
                "all in-window points should be counted"
            );
            assert!(time_span_hours >= 72, "72h requirement must be satisfied");
        }
        other => panic!("expected sufficient snapshot status, found {:?}", other),
    }
}

/// Document the behaviour for 96-hour requests: with three 128-entry chunks the
/// time span narrowly misses 96h due to 15-minute granularity. The snapshot
/// logic should therefore flag this as insufficient, providing a safety margin
/// rather than silently passing degraded data.
///
/// Why this is conservative:
/// - The system intentionally errs on the side of rejecting marginal history
///   that does not clearly meet the requested window. This prevents subtle
///   acceptance of under-sampled data which could be exploited or lead to
///   degraded economic decisions.
#[test]
fn ninety_six_hour_requirement_flags_time_span_gap() {
    let oracle_state: OracleState = minimal_oracle_state();
    let total_points = BUFFER_SIZE * 3; // 384 entries across three chunks
    let current_seed = 0i64;
    let chunks = build_historical_span(total_points, current_seed);
    let current_timestamp = super::helpers::deterministic_price_point(current_seed).timestamp;

    let status =
        oracle_state.check_snapshot_requirements_from_history(&chunks, current_timestamp, 96);
    match status {
        SnapshotStatus::InsufficientTimeSpan {
            span_hours,
            required_hours,
        } => {
            assert!(
                span_hours < required_hours,
                "span should document the shortfall"
            );
            assert_eq!(required_hours, 96);
        }
        other => panic!(
            "expected insufficient time span for 96h check, got {:?}",
            other
        ),
    }
}

/// Demonstrates timestamp filtering: points older than the requested window are
/// excluded, ensuring the validator does not accidentally include stale data in
/// the sufficiency count.
///
/// Note on granularity:
/// - Historical snapshots are sampled at discrete intervals (e.g., 15 minutes).
///   The count-based logic must therefore combine both time-span and count
///   checks to avoid off-by-one acceptance around window edges. This test
///   ensures older points outside the window are not counted even if they are
///   present in adjacent chunks.
#[test]
fn timestamp_filtering_discards_out_of_window_points() {
    let oracle_state: OracleState = minimal_oracle_state();

    // Build history covering 48 hours ending at `current_seed` = 0.
    let total_points = 193; // (48h * 4) + 1 ensures >= 48h span
    let current_seed = 0i64;
    let chunks = build_historical_span(total_points, current_seed);

    let current_timestamp = super::helpers::deterministic_price_point(current_seed).timestamp;
    let required_hours = 24u16;
    let window_seconds = (required_hours as i64) * SECONDS_PER_HOUR;
    let status = oracle_state.check_snapshot_requirements_from_history(
        &chunks,
        current_timestamp,
        required_hours,
    );
    match status {
        SnapshotStatus::Sufficient { snapshot_count, .. } => {
            // Only the last 24 hours (plus the +1 to hit full span) should be counted.
            let expected = ((window_seconds / MIN_HISTORICAL_INTERVAL) as usize) + 1;
            assert_eq!(
                snapshot_count, expected as u16,
                "older snapshots must be excluded"
            );
        }
        other => panic!("expected sufficient status for 24h window, got {:?}", other),
    }
}
