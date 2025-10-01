//! Property-based tests that hammer the circular buffer using randomised input
//! sequences. These catch edge cases that hand-written unit tests might miss.

use super::helpers::{
    assert_chunk_invariants, assert_price_point_eq, collect_fifo_view, empty_chunk,
    proptest_price_point_strategy, BUFFER_SIZE_U16,
};
use crate::utils::constants::BUFFER_SIZE;
use proptest::collection::vec;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, max_shrink_iters: 100, .. ProptestConfig::default() })]
    /// Random push sequences should never violate buffer invariants. This test
    /// ensures pointer arithmetic stays within bounds regardless of input
    /// patterns, defending against panic-triggering wraparound bugs.
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

    /// Randomised timestamp/price combinations should never cause panics and
    /// must keep the buffer logically full once capacity is reached. This test
    /// defends against arithmetic overflows that could modify `count`.
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
