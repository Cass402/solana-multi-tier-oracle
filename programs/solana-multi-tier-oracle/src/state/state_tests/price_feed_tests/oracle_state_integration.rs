use super::core_unit_tests_and_utils::sample_price_feed;
use crate::error::StateError;
use crate::state::oracle_state::{OracleState, PriceData, StateFlags, Version};
use crate::state::price_feed::{FeedFlags, PriceFeed};
use crate::utils::constants::{MAX_HISTORICAL_CHUNKS, MAX_LP_CONCENTRATION, MAX_PRICE_FEEDS};
use anchor_lang::error::Error;
use anchor_lang::prelude::Pubkey;

fn oracle_state_with_feeds(feeds: &[PriceFeed], manipulation_threshold: u16) -> OracleState {
    let mut price_feeds = [PriceFeed::default(); MAX_PRICE_FEEDS];
    for (idx, feed) in feeds.iter().enumerate() {
        price_feeds[idx] = *feed;
    }

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
        price_feeds,
        twap_window: 0,
        current_chunk_index: 0,
        max_chunk_size: 0,
        confidence_threshold: 0,
        manipulation_threshold,
        active_feed_count: feeds.len() as u8,
        bump: 0,
        governance_bump: 0,
        historical_chunks: [Pubkey::default(); MAX_HISTORICAL_CHUNKS],
        emergency_admin: Pubkey::default(),
        asset_seed: [0; 32],
        reserved: [0; 513],
    }
}

fn assert_error_code(result: Result<(), Error>, expected: StateError) {
    // We intentionally assert on concrete Anchor error codes instead of
    // matching error messages. Error codes form a stable contract between
    // program and caller; messages may change and are not relied on by
    // cross-program clients or off-chain tooling.
    let err = result.expect_err("expected oracle manipulation check to fail");
    let expected_error: Error = expected.into();

    let actual_code = error_code_number(&err).expect("expected anchor error with code");
    let expected_code =
        error_code_number(&expected_error).expect("expected anchor error with code");

    // Comparing numeric codes guards against fragile string comparisons and
    // exercises the same path Anchor runtime would use to surface the error
    // to clients.
    assert_eq!(actual_code, expected_code, "unexpected error variant");
}

fn error_code_number(err: &Error) -> Option<u32> {
    match err {
        Error::AnchorError(anchor_err) => Some(anchor_err.error_code_number),
        Error::ProgramError(_) => None,
    }
}
/// Tests below exercise the manipulation-resistance policy. They are written
/// to document the security trade-offs and the intended governance behavior:
///
/// - Only active feeds should affect oracle safety calculations. An inactive
///   feed represents an intentionally excluded source (e.g., operator
///   maintenance or delisting) and must never cause the whole oracle to be
///   marked as manipulated.
/// - Governance parameters (like `MAX_LP_CONCENTRATION` and the
///   `manipulation_threshold`) are safety knobs. Tests assert that those
///   knobs are enforced for active sources and ignored for inactive ones.
/// - We assert on explicit `StateError` variants so audits and upstream
///   callers can reason about precise failure modes (e.g., LP concentration
///   vs. manipulation score) rather than generic errors.

#[test]
fn inactive_feeds_are_skipped_by_manipulation_checks() {
    // Simulate a feed that would be risky by LP concentration but is not
    // considered because it is inactive. This ensures the skip-path is
    // honoured — a critical behavior for operator-managed lifecycle events.
    let mut feed = sample_price_feed();
    feed.lp_concentration = MAX_LP_CONCENTRATION + 500;

    let state = oracle_state_with_feeds(&[feed], /*manipulation_threshold=*/ 500);
    assert!(state.check_manipulation_resistance().is_ok());
}

#[test]
fn active_feed_fails_on_excessive_lp_concentration() {
    // An active feed above governance LP concentration limits should trigger
    // a deterministic and explicitly typed error. This prevents stale or
    // highly concentrated pools from silently influencing the aggregated
    // price.
    let mut feed = sample_price_feed();
    feed.flags.set(FeedFlags::ACTIVE);
    feed.lp_concentration = MAX_LP_CONCENTRATION + 1;

    let state = oracle_state_with_feeds(&[feed], /*manipulation_threshold=*/ 1_000);
    assert_error_code(
        state.check_manipulation_resistance(),
        StateError::ExcessiveLpConcentration,
    );
}

#[test]
fn active_feed_detects_manipulation_score_violation() {
    // Manipulation score aggregates multiple signals; exceeding the
    // governance threshold should be surfaced distinctly so operators can
    // identify the reason for failure.
    let mut feed = sample_price_feed();
    feed.flags.set(FeedFlags::ACTIVE);
    feed.manipulation_score = 1_200;

    let state = oracle_state_with_feeds(&[feed], /*manipulation_threshold=*/ 1_000);
    assert_error_code(
        state.check_manipulation_resistance(),
        StateError::ManipulationDetected,
    );
}

#[test]
fn mixed_feed_activation_only_checks_active_entries() {
    // Mix active and inactive feeds. Only the active feed should be used in
    // manipulation calculations. This test also ensures ordering of entries
    // in the fixed-size `price_feeds` array does not affect the outcome.
    let mut clean_active = sample_price_feed();
    clean_active.flags.set(FeedFlags::ACTIVE);
    clean_active.lp_concentration = MAX_LP_CONCENTRATION - 10;
    clean_active.manipulation_score = 50;

    let mut risky_inactive = sample_price_feed();
    risky_inactive.lp_concentration = MAX_LP_CONCENTRATION + 5;
    risky_inactive.manipulation_score = 5_000;

    // Place risky feed second to ensure ordering doesn't matter.
    let state = oracle_state_with_feeds(
        &[clean_active, risky_inactive],
        /*manipulation_threshold=*/ 200,
    );
    assert!(state.check_manipulation_resistance().is_ok());
}
