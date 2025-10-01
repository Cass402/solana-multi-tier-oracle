/// Core unit tests and utilities for price feed functionality validation.
///
/// # Testing Philosophy
///
/// These tests focus on critical invariants and edge cases that could compromise oracle
/// integrity in production. Each test validates specific safety properties that prevent
/// data corruption, type confusion, or manipulation vulnerabilities.
///
/// # Data Integrity Focus
///
/// Particular emphasis on enum serialization safety, flag manipulation correctness,
/// and defensive programming patterns that ensure the oracle maintains consistent
/// state even when receiving malformed or malicious inputs.
use crate::state::price_feed::{FeedFlags, PriceFeed, SourceType};
use anchor_lang::prelude::Pubkey;

/// Factory function for creating realistic PriceFeed test instances with production-like values.
///
/// # Test Data Strategy
///
/// Uses realistic market values rather than simple incremental numbers to catch edge cases
/// that might only manifest with real-world data ranges. The values represent a typical
/// DEX price feed with moderate liquidity and low manipulation risk.
///
/// # Field Selection Rationale
///
/// - `last_price: 42`: Mid-range value that avoids overflow edge cases
/// - `volume_24h: 1_000`: Moderate trading volume indicating active but not excessive activity
/// - `liquidity_depth: 50_000`: Sufficient depth to resist manipulation but not unrealistically high
/// - `last_conf: 25`: 0.25% confidence interval representing typical DeFi price uncertainty
/// - `last_expo: -6`: Standard 6 decimal precision common in DeFi tokens
/// - `manipulation_score: 500`: Low-medium risk score (5% on 0-10000 scale)
///
/// # Defensive Initialization
///
/// All fields explicitly initialized to prevent undefined behavior from partial initialization,
/// critical for zero-copy deserialization safety in the Solana runtime environment.
pub(crate) fn sample_price_feed() -> PriceFeed {
    PriceFeed {
        source_address: Pubkey::new_unique(),
        last_price: 42,
        volume_24h: 1_000,
        liquidity_depth: 50_000,
        last_conf: 25,
        last_update: 1_700_000_000, // Realistic unix timestamp (2023-11-14)
        last_expo: -6,
        weight: 5_000,           // 50% weight in aggregation (basis points)
        lp_concentration: 1_000, // 10% LP concentration
        manipulation_score: 500, // 5% manipulation risk
        source_type: SourceType::DEX.as_u8(),
        flags: FeedFlags::new(),
        _padding: [0; 4],
    }
}

/// Validates enum serialization consistency for cross-program compatibility.
///
/// # Serialization Safety Critical Test
///
/// Ensures SourceType discriminants remain stable across program updates, preventing
/// deserialization failures when oracle data is consumed by external programs.
/// Changes to these discriminant values would break existing on-chain data.
///
/// # Zero-Copy Deserialization Requirements
///
/// The oracle uses zero-copy patterns where enum discriminants are directly read
/// from account memory. This test validates that the discriminant mapping remains
/// consistent, preventing type confusion that could lead to incorrect price source
/// interpretation and potential manipulation vulnerabilities.
#[test]
fn source_type_as_u8_roundtrip() {
    // Test each enum variant's discriminant stability
    // These values are effectively part of the on-chain data format contract
    for (variant, discriminant) in [
        (SourceType::DEX, 0u8),
        (SourceType::CEX, 1u8),
        (SourceType::Oracle, 2u8),
        (SourceType::Aggregator, 3u8),
    ] {
        // Forward conversion must be deterministic for consistent serialization
        assert_eq!(
            variant.as_u8(),
            discriminant,
            "round-trip discriminant mismatch"
        );

        // Reverse conversion must succeed for zero-copy deserialization safety
        assert_eq!(
            SourceType::from_u8(discriminant),
            Some(variant),
            "from_u8 failed round-trip"
        );
    }
}

/// Validates defensive deserialization against malformed or malicious data.
///
/// # Attack Vector Prevention
///
/// Tests that invalid discriminant values are properly rejected rather than causing
/// undefined behavior or silent data corruption. This is critical for oracle security
/// as malicious actors might attempt to inject accounts with invalid enum values.
///
/// # Memory Safety Considerations
///
/// In zero-copy deserialization, invalid enum discriminants could cause undefined
/// behavior if not properly validated. This test ensures the oracle fails safely
/// when encountering corrupted or maliciously crafted account data.
#[test]
fn source_type_from_u8_rejects_invalid() {
    // Test boundary case: maximum u8 value should be rejected
    assert_eq!(SourceType::from_u8(255), None);

    // Test just beyond valid range: first invalid discriminant should be rejected
    assert_eq!(SourceType::from_u8(4), None);
}

/// Validates fail-safe behavior for corrupted data recovery.
///
/// # Defensive Programming Strategy
///
/// When encountering invalid discriminants, the oracle falls back to the most
/// conservative source type (DEX) rather than failing completely. This approach
/// maintains oracle availability while applying the most restrictive trust assumptions.
///
/// # Risk Mitigation Rationale
///
/// DEX sources typically have the lowest trust level and highest manipulation
/// resistance requirements, making them the safest default when data integrity
/// is questionable. This prevents oracle shutdown while maintaining security.
#[test]
fn source_type_from_u8_or_default_fallbacks_to_dex() {
    // Invalid discriminant should fall back to most conservative source type
    assert_eq!(SourceType::from_u8_or_default(200), SourceType::DEX);
}

/// Validates type-safe source type mutation interface.
///
/// # API Safety Guarantees
///
/// Tests the abstraction layer that prevents direct manipulation of the raw u8
/// discriminant field, ensuring all source type changes go through validated
/// setters that maintain data integrity invariants.
///
/// # Encapsulation Benefits
///
/// The getter/setter pattern prevents accidental corruption of the source_type
/// field while providing a clean interface for legitimate updates. This is
/// especially important in zero-copy contexts where field access bypasses
/// normal Rust safety checks.
#[test]
fn price_feed_source_type_get_set() {
    let mut feed = sample_price_feed();
    // Verify initial state matches factory default
    assert!(feed.is_source_type(SourceType::DEX));

    // Test mutation through type-safe interface
    feed.set_source_type(SourceType::Aggregator);

    // Verify all access methods return consistent results
    assert_eq!(feed.get_source_type(), SourceType::Aggregator);
    assert!(feed.is_source_type(SourceType::Aggregator));
    assert!(!feed.is_source_type(SourceType::DEX));
}

/// Validates bitwise flag manipulation correctness and atomicity.
///
/// # Bitfield Safety Critical Test
///
/// Ensures flag operations maintain bit-level precision without affecting
/// adjacent flags. Incorrect bitwise operations could corrupt multiple flags
/// simultaneously, leading to oracle state inconsistencies.
///
/// # Performance Optimization Context
///
/// Uses bitwise operations instead of boolean fields to minimize memory usage
/// and enable atomic flag updates in zero-copy scenarios. This test validates
/// that the optimization doesn't introduce correctness bugs.
///
/// # State Machine Validation
///
/// Tests the fundamental state transitions that control oracle behavior,
/// ensuring flags can be reliably set, cleared, and toggled without
/// side effects on other system state.
#[test]
fn feed_flags_set_clear_toggle_paths() {
    let mut flags = FeedFlags::new();
    // Verify clean initial state
    assert!(!flags.is_active());

    // Test set operation and verify both access methods
    flags.set(FeedFlags::ACTIVE);
    assert!(flags.is_active());
    assert!(flags.has(FeedFlags::ACTIVE)); // Verify both APIs return consistent results

    // Test clear operation returns to initial state
    flags.clear(FeedFlags::ACTIVE);
    assert!(!flags.is_active());

    // Test toggle operation for stateful flag management
    flags.toggle(FeedFlags::TRUSTED);
    assert!(flags.is_trusted());
    flags.toggle(FeedFlags::TRUSTED);
    assert!(!flags.is_trusted()); // Should return to initial state
}

/// Validates conditional flag setting for algorithmic state management.
///
/// # API Convenience and Safety
///
/// Tests the set_to() method that enables conditional flag updates based on
/// computed boolean values, commonly used in manipulation detection algorithms
/// where flag states depend on complex calculations.
///
/// # Atomic Operation Equivalence
///
/// Ensures set_to(flag, condition) produces identical results to manual
/// if/else branching with set()/clear() calls, but with better performance
/// and atomicity guarantees in concurrent scenarios.
///
/// # Manipulation Detection Context
///
/// Uses MANIPULATION_DETECTED flag as test subject since this flag is frequently
/// updated by algorithmic processes that compute risk scores and need reliable
/// conditional setting based on threshold comparisons.
#[test]
fn feed_flags_set_to_behaviour() {
    let mut flags = FeedFlags::new();

    // Test conditional setting to true
    flags.set_to(FeedFlags::MANIPULATION_DETECTED, true);
    assert!(flags.is_manipulation_detected());

    // Test conditional setting to false
    flags.set_to(FeedFlags::MANIPULATION_DETECTED, false);
    assert!(!flags.is_manipulation_detected());
}

/// Validates defensive deserialization against flag corruption and future compatibility.
///
/// # Forward Compatibility Strategy
///
/// Tests that unknown flag bits are silently discarded rather than causing
/// deserialization failures. This enables the oracle to safely read data
/// created by newer versions that may have additional flags defined.
///
/// # Data Integrity Protection
///
/// Ensures the VALID_MASK correctly filters out potentially malicious or
/// corrupted bits that could affect oracle behavior if interpreted as
/// valid flags. This prevents flag-based attacks through data manipulation.
///
/// # Zero-Copy Safety
///
/// In zero-copy deserialization, raw bytes are directly interpreted as flag
/// values. This test ensures the filtering mechanism works correctly to
/// prevent undefined behavior from invalid bit patterns in account data.
#[test]
fn feed_flags_from_u8_truncate_filters_unknown_bits() {
    // Inject invalid bits in upper positions to simulate corruption or future flags
    let raw = FeedFlags::ACTIVE.as_u8() | 0b1110_0000;
    let filtered = FeedFlags::from_u8_truncate(raw);

    // Verify known flag is preserved
    assert!(filtered.is_active());

    // Verify unknown bits are completely filtered out
    assert_eq!(filtered.as_u8() & !FeedFlags::VALID_MASK, 0);
}
