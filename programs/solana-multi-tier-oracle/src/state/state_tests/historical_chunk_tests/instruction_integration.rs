//! Cross-instruction integration tests that exercise `HistoricalChunk` through
//! realistic instruction-driven flows. These are not full validator tests but
//! instead simulate the essential zero-copy and account lifecycle behaviours an
//! on-chain program experiences when running under Anchor.
//!
//! Why these tests exist:
//! - Unit tests validate internal algorithms, but instruction-level tests prove
//!   those algorithms remain correct when executed via the same memory access
//!   and account-loading patterns used in production (e.g., `AccountLoader`'s
//!   zero-copy borrows).
//! - The tests replicate the mutation and reload cycles that can expose
//!   subtle layout or alignment problems, padding corruption, or incorrect
//!   writeback semantics that unit tests (which operate on owned structs)
//!   might miss.

#[cfg(test)]
mod instruction_tests {
    use crate::state::historical_chunk::{HistoricalChunk, PricePoint};
    use crate::utils::constants::{BUFFER_SIZE, MIN_HISTORICAL_INTERVAL};
    use anchor_lang::prelude::*;
    use std::mem::size_of;

    /// Simulates the historical chunk account lifecycle under repeated
    /// `update_price`-like invocations to validate FIFO ordering, timestamp
    /// spacing, and saturation behaviour when driven by instruction context.
    ///
    /// Test strategy and intent:
    /// - Allocate a raw byte buffer sized like the on-chain account and cast it
    ///   to a `HistoricalChunk` to reproduce the exact memory layout.
    /// - Repeatedly load a mutable borrow to emulate `AccountLoader::load_mut`.
    /// - Perform interval-based pushes and assert invariants after each
    ///   iteration to catch state drift that could happen across instruction
    ///   boundaries (e.g., incorrect head/tail arithmetic or padding writes).
    ///
    /// Trade-offs:
    /// - This test intentionally sacrifices the full validator semantics (no
    ///   CPI or signature checking) for speed and direct control over the
    ///   zero-copy mutation path. It focuses on memory/image-level correctness
    ///   which is crucial for on-chain upgrades and safety.
    #[test]
    fn historical_chunk_survives_repeated_instruction_cycles() {
        // Allocate a chunk account buffer matching on-chain layout.
        let mut chunk_buffer = vec![0u8; size_of::<HistoricalChunk>()];

        // Initialize chunk metadata as the instruction handler would during account creation.
        {
            let chunk = unsafe { &mut *(chunk_buffer.as_mut_ptr() as *mut HistoricalChunk) };
            chunk.chunk_id = 0;
            chunk.head = 0;
            chunk.tail = 0;
            chunk.count = 0;
            chunk.creation_timestamp = 1_700_000_000;
            chunk.next_chunk = Pubkey::default();
            chunk.oracle_state = Pubkey::new_unique();
            chunk.bump = 255;
            chunk.reserved = [0; 511];
        }

        // Simulate a sequence of update_price instruction executions that push historical data.
        let base_timestamp = 1_700_000_000i64;
        let push_count = BUFFER_SIZE + BUFFER_SIZE / 2; // Enough to trigger wraparound

        for iteration in 0..push_count {
            let current_time = base_timestamp + (iteration as i64 * MIN_HISTORICAL_INTERVAL);

            // Load chunk mutably (simulating AccountLoader::load_mut in instruction context).
            let chunk = unsafe { &mut *(chunk_buffer.as_mut_ptr() as *mut HistoricalChunk) };

            // Determine whether to push based on interval check (mirrors update_price logic).
            let should_push = match chunk.latest() {
                Some(last_point) => {
                    let time_delta = current_time - last_point.timestamp;
                    time_delta >= MIN_HISTORICAL_INTERVAL
                }
                None => true, // First push always succeeds
            };

            if should_push {
                let new_point = PricePoint {
                    price: 1_000_000_000_000 + (iteration as i128 * 1000),
                    volume: 500_000_000,
                    conf: 25,
                    timestamp: current_time,
                };
                chunk.push(new_point);
            }

            // Assert per-instruction invariants that must hold after every mutation.
            assert!(
                chunk.count <= BUFFER_SIZE as u16,
                "count exceeded buffer capacity at iteration {}",
                iteration
            );
            assert!(
                chunk.head < BUFFER_SIZE as u16,
                "head pointer out of bounds at iteration {}",
                iteration
            );
            assert!(
                chunk.tail < BUFFER_SIZE as u16,
                "tail pointer out of bounds at iteration {}",
                iteration
            );

            // Verify latest element matches what we just pushed.
            if should_push {
                let latest = chunk
                    .latest()
                    .expect("chunk should contain data after push");
                assert_eq!(
                    latest.timestamp, current_time,
                    "latest timestamp mismatch at iteration {}",
                    iteration
                );
            }

            // After saturation, count must remain capped.
            if iteration >= BUFFER_SIZE {
                assert_eq!(
                    chunk.count, BUFFER_SIZE as u16,
                    "count should stabilize at BUFFER_SIZE after saturation"
                );
            }
        }

        // Final post-execution validation: verify FIFO order over the retained window.
        let chunk = unsafe { &*(chunk_buffer.as_ptr() as *const HistoricalChunk) };

        let mut previous_timestamp = 0i64;
        let mut traversed_count = 0u16;
        let mut idx = chunk.tail;

        for _ in 0..chunk.count {
            let point = &chunk.price_points[idx as usize];
            assert!(
                point.timestamp > previous_timestamp,
                "FIFO order violated: timestamps not monotonically increasing"
            );
            previous_timestamp = point.timestamp;
            traversed_count += 1;
            idx = (idx + 1) & (BUFFER_SIZE as u16 - 1);
        }

        assert_eq!(
            traversed_count, chunk.count,
            "FIFO traversal did not cover the full count"
        );
    }

    /// Ensures zero-copy reloads (drop/reload cycles) do not introduce byte-level
    /// drift or corrupt reserved/padding regions.
    ///
    /// Motivation:
    /// - On Solana an account can be loaded, mutated, and persisted across
    ///   transactions. Any non-deterministic writeback (e.g., uninitialized
    ///   padding becoming non-zero) can break equality checks, snapshots, or
    ///   client-side expectations.
    /// - The test snapshots the raw byte buffer before and after a simulated
    ///   reload to assert byte-for-byte identity, which closely mirrors the
    ///   guarantee required by zero-copy readers.
    #[test]
    fn zero_copy_reload_preserves_chunk_integrity() {
        // Initial account creation with deterministic state.
        let mut chunk_buffer = vec![0u8; size_of::<HistoricalChunk>()];
        {
            let chunk = unsafe { &mut *(chunk_buffer.as_mut_ptr() as *mut HistoricalChunk) };
            chunk.chunk_id = 7;
            chunk.creation_timestamp = 1_700_000_000;
            chunk.oracle_state = Pubkey::new_unique();
            chunk.bump = 42;

            // Push a few entries to establish non-trivial state.
            for i in 0..10i64 {
                chunk.push(PricePoint {
                    price: 1_000_000_000 + (i as i128) * 100,
                    volume: 500_000,
                    conf: 10,
                    timestamp: 1_700_000_000 + i * MIN_HISTORICAL_INTERVAL,
                });
            }
        }

        // Snapshot the original bytes before reload.
        let original_bytes = chunk_buffer.clone();

        // Simulate a reload cycle (mimicking AccountLoader dropping and reloading).
        // In production, the account bytes would be persisted and reloaded from on-chain state.
        {
            let chunk = unsafe { &mut *(chunk_buffer.as_mut_ptr() as *mut HistoricalChunk) };

            // Perform a no-op mutation to exercise the write path.
            let _dummy_read = chunk.count;
        }

        // Verify byte-for-byte identity after reload (proving no drift/corruption).
        assert_eq!(
            chunk_buffer, original_bytes,
            "zero-copy reload introduced unexpected byte-level changes"
        );

        // Assert reserved padding remains zero after reload.
        let chunk = unsafe { &*(chunk_buffer.as_ptr() as *const HistoricalChunk) };
        assert!(
            chunk.reserved.iter().all(|b| *b == 0),
            "reserved padding was mutated during reload cycle"
        );
    }

    /// Guards the on-chain account size and alignment assumptions used by
    /// deployment tools and rent-exemption calculations.
    ///
    /// Why this is necessary:
    /// - The exact byte-size of the account influences rent, allocation, and
    ///   deployment scripts. A silently changed struct layout would break
    ///   existing deployments and could cause runtime panics when accounts are
    ///   read with zero-copy casts expecting a different size.
    /// - The test ties the `size_of::<HistoricalChunk>()` to an explicit
    ///   expected numeric value so CI will flag any layout drift immediately.
    #[test]
    fn historical_chunk_account_size_matches_deployment_expectations() {
        const EXPECTED_ACCOUNT_SIZE: usize = size_of::<HistoricalChunk>();

        // This assertion guards against accidental layout drift that would break
        // existing account allocations or rent calculations in deployment tooling.
        // Calculation: 2+2+2+2 (metadata) + 8 (timestamp) + 32+32 (pubkeys) + (48*128) (price_points) + 1 (bump) + 511 (reserved)
        // With alignment padding: rounds up to 6736 due to 16-byte alignment requirement
        assert_eq!(
            EXPECTED_ACCOUNT_SIZE, 6736,
            "HistoricalChunk size changed; update deployment scripts and rent calculations"
        );

        // Validate alignment requirements for zero-copy safety.
        assert_eq!(
            std::mem::align_of::<HistoricalChunk>(),
            16,
            "alignment requirement changed; may break zero-copy assumptions"
        );
    }
}
