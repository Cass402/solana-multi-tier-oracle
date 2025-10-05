//! Property-based tests that aggressively exercise the circular buffer using
//! randomized input sequences. The goal here is to exercise edge cases that
//! deterministic unit tests may miss—particularly around wraparound arithmetic,
//! index masking, and signed arithmetic boundaries that can cause panics or
//! silently incorrect state.
//!
//! Why property tests:
//! - Unit tests assert specific, expected behaviors. Property tests instead
//!   assert invariants that must always hold regardless of input order or
//!   magnitude. This helps catch subtle regressions introduced by refactors.
//! - The circular buffer relies heavily on bitmask arithmetic and fixed-size
//!   indices; randomized sequences increase confidence that pointer math is
//!   robust under a wide range of inputs.

use super::helpers::{
    assert_chunk_invariants, assert_price_point_eq, collect_fifo_view, empty_chunk,
    proptest_price_point_strategy, BUFFER_SIZE_U16,
};
use crate::utils::constants::BUFFER_SIZE;
use proptest::collection::vec;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, max_shrink_iters: 100, .. ProptestConfig::default() })]
    /// Property: random push sequences never violate core buffer invariants.
    ///
    /// Rationale:
    /// - The circular buffer uses wraparound masking and fixed-size counters.
    ///   Off-by-one arithmetic or incorrect masking often only surfaces when
    ///   inputs hit unusual sequences — randomized testing helps find those
    ///   sequences.
    /// - We assert both structural invariants (head/tail/count ranges) and
    ///   behavioural invariants (latest element equals the last pushed item)
    ///   to ensure both memory safety and semantic correctness.
    fn random_push_sequences_preserve_invariants(points in vec(proptest_price_point_strategy(), 0..512)) {
        let mut chunk = empty_chunk();

        for point in points.iter().copied() {
            let prev_count = chunk.count;
            let prev_tail = chunk.tail;
            chunk.push(point);
            assert_chunk_invariants!(chunk);

            if prev_count < BUFFER_SIZE_U16 {
                assert_eq!(chunk.tail, prev_tail, "tail must not advance before saturation");
            } else {
                assert_eq!(chunk.tail, (prev_tail + 1) & (BUFFER_SIZE_U16 - 1), "tail advances exactly one step under saturation");
            }

            let latest = chunk.latest().expect("latest should be available after first push");
            assert_price_point_eq(latest, &point);
        }

        let expected_count = std::cmp::min(points.len(), BUFFER_SIZE) as u16;
        assert_eq!(chunk.count, expected_count);

        let fifo = collect_fifo_view(&chunk);
        if points.len() <= BUFFER_SIZE {
            assert_eq!(fifo.len(), points.len());
            for (actual, expected) in fifo.iter().zip(points.iter()) {
                assert_price_point_eq(actual, expected);
            }
        } else {
            let tail_slice = &points[points.len() - BUFFER_SIZE..];
            assert_eq!(fifo.len(), tail_slice.len());
            for (actual, expected) in fifo.iter().zip(tail_slice.iter()) {
                assert_price_point_eq(actual, expected);
            }
        }
    }

    /// Property: long sequences of randomized inputs must cap `count` at the
    /// configured buffer size and avoid panics.
    ///
    /// Safety focus:
    /// - This test targets overflow or wraparound bugs in arithmetic used to
    ///   update `count`, `head`, and `tail`. By pushing between `BUFFER_SIZE`
    ///   and `2*BUFFER_SIZE` items we ensure saturation behaviour stabilizes
    ///   and that the latest element remains accessible.
    fn randomised_inputs_cap_count(points in vec(proptest_price_point_strategy(), BUFFER_SIZE..BUFFER_SIZE*2)) {
        let mut chunk = empty_chunk();
        for point in points.iter().copied() {
            chunk.push(point);
        }

        assert_eq!(chunk.count, BUFFER_SIZE_U16, "count must clamp at BUFFER_SIZE despite long runs");
        assert!(chunk.latest().is_some());
        assert_chunk_invariants!(chunk);
    }
}
