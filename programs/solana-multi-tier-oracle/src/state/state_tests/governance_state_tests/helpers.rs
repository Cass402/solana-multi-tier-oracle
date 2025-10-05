use crate::error::StateError;
use crate::state::governance_state::{GovernanceState, Permissions};
use crate::utils::constants::{MAX_ALLOWED_PROGRAMS, MAX_MULTISIG_MEMBERS};
use anchor_lang::error::Error;
use anchor_lang::prelude::{Pubkey, Result as AnchorResult};
use std::mem::{size_of, MaybeUninit};
use std::ptr;

// Test helpers for governance_state unit tests.
//
// Notes on intent:
// - These helpers create deterministic, zero-surprise fixtures meant for
//   reasoning about security invariants. Fixtures must be reproducible so
//   fuzzing, property tests, and auditors can replicate failing scenarios.
// - The helpers deliberately use fixed-size arrays and zero-copy patterns to
//   mirror on-chain constraints (no Vec, stable layout, explicit padding).
// - Where we construct raw bytes or reconstruct structs from bytes we do so to
//   emulate the memory layout of on-chain accounts and validate ABI/serialization
//   assumptions.

/// Default number of allowed DEX programs populated in fixtures.
///
/// Rationale: using a small, non-zero default helps tests exercise slice
/// truncation and capacity-related invariants without needing a full-capacity
/// array. Keeps test data lightweight while still validating boundary
/// behaviours.
pub(crate) const DEFAULT_ALLOWED_DEX: usize = 3;
/// Default number of allowed aggregator programs populated in fixtures.
pub(crate) const DEFAULT_ALLOWED_AGGREGATORS: usize = 2;

/// Canonical list of permission atoms used across tests.
///
/// Why this exists: tests need a repeatable mapping from member slot to
/// permission pattern so assertions about additive/revocation semantics can be
/// deterministic. Using a small, representative set of permissions exercises
/// bitfield masks and collision behaviours without being exhaustive.
pub(crate) const PERMISSION_VARIANTS: [Permissions; 7] = [
    Permissions::UPDATE_PRICE,
    Permissions::TRIGGER_CIRCUIT_BREAKER,
    Permissions::MODIFY_CONFIG,
    Permissions::VIEW_METRICS,
    Permissions::EMERGENCY_HALT,
    Permissions::ADD_FEED,
    Permissions::REMOVE_FEED,
];

/// Generates a deterministic, non-default pubkey based on a simple seed.
///
/// Rationale: deterministic keys make test failures reproducible. These keys
/// are intentionally non-zero to avoid colliding with `Pubkey::default()` and
/// to exercise code paths that skip default/empty slots.
pub(crate) fn deterministic_pubkey(seed: u8) -> Pubkey {
    let mut bytes = [0u8; 32];
    for (idx, byte) in bytes.iter_mut().enumerate() {
        *byte = seed
            .wrapping_add((idx as u8).wrapping_mul(37))
            .wrapping_add(1);
    }
    Pubkey::new_from_array(bytes)
}

/// Produces a governance state fixture with predictable fields and permission layout.
///
/// Design choices in the fixture:
/// - Uses fixed-size arrays to mirror on-chain account layout; avoids heap
///   allocations to better reflect program constraints.
/// - Sets realistic timing/threshold values so logic that depends on these
///   fields behaves similarly to production code during tests.
/// - Populates `multisig_members` and `member_permissions` deterministically so
///   membership-related invariants can be asserted without flakiness.
pub(crate) fn governance_fixture(active_members: u8) -> GovernanceState {
    assert!(active_members as usize <= MAX_MULTISIG_MEMBERS);

    let mut state = GovernanceState {
        proposal_threshold: 42,
        voting_period: 48 * 60 * 60,
        execution_delay: 12 * 60 * 60,
        timelock_duration: 72 * 60 * 60,
        veto_period: 24 * 60 * 60,
        quorum_threshold: 6_000,
        multi_sig_threshold: active_members.max(1),
        active_member_count: active_members,
        bump: 255,
        strict_mode_enabled: 0,
        allowed_dex_program_count: DEFAULT_ALLOWED_DEX as u8,
        allowed_aggregator_program_count: DEFAULT_ALLOWED_AGGREGATORS as u8,
        allowed_dex_programs: [Pubkey::default(); MAX_ALLOWED_PROGRAMS],
        allowed_aggregator_programs: [Pubkey::default(); MAX_ALLOWED_PROGRAMS],
        oracle_state: deterministic_pubkey(200),
        multisig_members: [Pubkey::default(); MAX_MULTISIG_MEMBERS],
        member_permissions: [Permissions::new(); MAX_MULTISIG_MEMBERS],
        reserved: [0; 512],
    };

    populate_allowed_programs(&mut state);
    populate_members(&mut state);
    state
}

fn populate_allowed_programs(state: &mut GovernanceState) {
    // Fill allowed program arrays up to a default count and leave the
    // remainder as `Pubkey::default()` to test slice/truncation invariants.
    for idx in 0..MAX_ALLOWED_PROGRAMS {
        state.allowed_dex_programs[idx] = if idx < DEFAULT_ALLOWED_DEX {
            deterministic_pubkey(40 + idx as u8)
        } else {
            Pubkey::default()
        };
        state.allowed_aggregator_programs[idx] = if idx < DEFAULT_ALLOWED_AGGREGATORS {
            deterministic_pubkey(80 + idx as u8)
        } else {
            Pubkey::default()
        };
    }
}

fn populate_members(state: &mut GovernanceState) {
    let member_count = state.active_member_count as usize;
    for idx in 0..MAX_MULTISIG_MEMBERS {
        // Populate active slots with deterministic keys and inactive slots
        // with `Pubkey::default()`. Tests rely on inactive slots being the
        // zero value to ensure find/member-index logic is robust.
        state.multisig_members[idx] = if idx < member_count {
            deterministic_pubkey(100 + idx as u8)
        } else {
            Pubkey::default()
        };
        state.member_permissions[idx] = if idx < member_count {
            PERMISSION_VARIANTS[idx % PERMISSION_VARIANTS.len()]
        } else {
            Permissions::new()
        };
    }
}

/// Helper asserting an Anchor error matches the expected `StateError` variant.
///
/// Rationale: Anchor wraps program errors in a variety of enums. Tests assert
/// the numeric error code to avoid fragile string matching and to precisely
/// validate the contract between the program's error types and the expected
/// `StateError` values.
pub(crate) fn assert_state_error(result: AnchorResult<()>, expected: StateError) {
    let err = result.expect_err("expected error result");
    let expected_error: Error = expected.into();

    // Extract numeric codes for deterministic comparison.
    let actual_code = error_code_number(&err).expect("expected anchor error code");
    let expected_code = error_code_number(&expected_error).expect("expected anchor error code");
    assert_eq!(actual_code, expected_code, "unexpected error variant");
}

fn error_code_number(err: &Error) -> Option<u32> {
    match err {
        Error::AnchorError(anchor_err) => Some(anchor_err.error_code_number),
        Error::ProgramError(_) => None,
    }
}

/// Converts a governance state into raw bytes mirroring on-chain account storage.
///
/// Safety and rationale:
/// - We use `ptr::copy_nonoverlapping` to produce an exact memory copy of the
///   `GovernanceState`. This is intentionally unsafe but necessary to emulate
///   how data would be laid out when serialized on-chain (zero-copy semantics).
/// - Tests using these bytes must ensure the `GovernanceState` layout is stable
///   (see separate layout/zero-copy tests). Any changes to struct layout must
///   be reflected in those tests.
pub(crate) fn governance_to_bytes(state: &GovernanceState) -> Vec<u8> {
    let mut bytes = vec![0u8; size_of::<GovernanceState>()];
    unsafe {
        ptr::copy_nonoverlapping(
            (state as *const GovernanceState) as *const u8,
            bytes.as_mut_ptr(),
            bytes.len(),
        );
    }
    bytes
}

/// Reconstructs a governance state from raw bytes produced by `governance_to_bytes`.
///
/// Safety and rationale:
/// - This uses `MaybeUninit` plus `ptr::copy_nonoverlapping` to avoid double
///   initialisation and maintain a precise byte-for-byte reconstruction. This
///   pattern is acceptable in tests when you control the input bytes but would
///   be dangerous in production code if bytes are untrusted.
pub(crate) fn governance_from_bytes(bytes: &[u8]) -> GovernanceState {
    assert_eq!(bytes.len(), size_of::<GovernanceState>());
    let mut uninit = MaybeUninit::<GovernanceState>::uninit();
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), uninit.as_mut_ptr() as *mut u8, bytes.len());
        uninit.assume_init()
    }
}

/// Convenience assertion ensuring reserved padding remains zeroed after mutations.
///
/// Why: zeroed padding is a contract for ABI stability. If padding is mutated it
/// often indicates a layout mismatch, uninitialised memory, or an off-by-one
/// write—each of which can lead to subtle and critical on-chain bugs.
pub(crate) fn assert_reserved_padding(state: &GovernanceState) {
    assert!(
        state.reserved.iter().all(|byte| *byte == 0),
        "reserved padding mutated unexpectedly"
    );
}

/// Validates that permission sets never contain bits outside the canonical mask.
///
/// Rationale: permission bits are stored compactly in a bitfield for gas
/// efficiency. Any unknown bit leaking into the set likely indicates a logic
/// error (e.g., shifts or arithmetic) that could grant unintended privileges.
pub(crate) fn assert_permissions_sanitized(perms: Permissions) {
    assert_eq!(
        perms.as_u64() & !Permissions::VALID_MASK,
        0,
        "permission set leaked unsupported bits"
    );
}

/// Extracts the active multisig member slice for ease of assertions.
pub(crate) fn active_member_slice(state: &GovernanceState) -> &[Pubkey] {
    &state.multisig_members[..state.active_member_count as usize]
}

/// Extracts active member permissions for assertion helpers.
pub(crate) fn active_permissions_slice(state: &GovernanceState) -> &[Permissions] {
    &state.member_permissions[..state.active_member_count as usize]
}
