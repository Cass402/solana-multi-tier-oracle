//! Deterministic, invariant-driven unit tests for `GovernanceState` primitives.
//!
//! Why these tests exist
//! - Governance primitives encode on-chain authority and must resist a wide
//!   range of accidental and adversarial behaviors. These tests express the
//!   security properties (invariants) we expect the public API to maintain.
//!
//! Focus and scope
//! - Tests operate at the public API surface rather than implementation
//!   internals, mirroring how external callers (and attackers) will interact
//!   with the program. This keeps the tests robust to internal refactors while
//!   preserving security guarantees.
//! - We validate three core themes:
//!   1. Membership invariants: active member count must be enforceable and
//!      reductions must revoke privileges.
//!   2. Permission invariants: bitwise permission operations must be additive,
//!      reversible, and constrained to a canonical mask to avoid accidental
//!      privilege escalation via unknown bits.
//!   3. Layout and ABI invariants: zero-copy and reserved padding must remain
//!      stable after state mutations so on-chain serialization never surprises
//!      downstream programs.
//!
//! Design trade-offs called out by the tests
//! - Zero-copy patterns and stack-allocated buffers reduce compute and heap
//!   pressure on-chain but require strict layout and padding checks — hence the
//!   dedicated padding/zeroing tests.
//! - Permission bits are represented as a compact bitfield for efficiency;
//!   tests focus on ensuring bit-twiddling doesn't introduce unknown bits or
//!   destroy unrelated permissions.

use super::helpers::{
    active_member_slice, active_permissions_slice, assert_reserved_padding, assert_state_error,
    deterministic_pubkey, governance_fixture,
};
use crate::error::StateError;
use crate::state::governance_state::Permissions;
use crate::utils::constants::{MAX_ALLOWED_PROGRAMS, MAX_MULTISIG_MEMBERS};
use anchor_lang::prelude::Pubkey;

#[test]
fn set_active_member_count_within_bounds_updates_state() {
    let mut state = governance_fixture(5);
    state
        .set_active_member_count(3)
        .expect("shrink within bounds");
    assert_eq!(state.active_member_count, 3);

    state
        .set_active_member_count(5)
        .expect("expand to original count");
    assert_eq!(state.active_member_count, 5);
}

#[test]
fn set_active_member_count_rejects_out_of_bounds() {
    let mut state = governance_fixture(5);
    assert_state_error(
        state.set_active_member_count((MAX_MULTISIG_MEMBERS + 1) as u8),
        StateError::TooManyActiveMembers,
    );
}

#[test]
fn grant_member_permission_updates_member_bitfield() {
    let mut state = governance_fixture(3);
    let original = state.member_permissions[1];

    state
        .grant_member_permission(1, Permissions::EMERGENCY_HALT)
        .expect("grant within range");

    assert!(state.member_permissions[1].can_emergency_halt());
    assert_eq!(
        state.member_permissions[1].as_u64() & original.as_u64(),
        original.as_u64(),
        "grant must be additive"
    );
}

#[test]
fn grant_member_permission_rejects_inactive_index() {
    let mut state = governance_fixture(2);
    state
        .set_active_member_count(1)
        .expect("shrink active set to single member");
    assert_state_error(
        state.grant_member_permission(1, Permissions::UPDATE_PRICE),
        StateError::UnauthorizedCaller,
    );
}

#[test]
fn revoke_member_permission_clears_only_target_bit() {
    let mut state = governance_fixture(2);
    let key_perm = Permissions::ADD_FEED;
    state
        .grant_member_permission(0, key_perm)
        .expect("prime permission prior to revoke");

    state
        .revoke_member_permission(0, key_perm)
        .expect("revoke within range");
    assert!(!state.member_permissions[0].can_add_feed());
    assert!(
        state.member_permissions[0].as_u64() & key_perm.as_u64() == 0,
        "target bit must be cleared"
    );
}

#[test]
fn revoke_member_permission_rejects_inactive_index() {
    let mut state = governance_fixture(3);
    state
        .set_active_member_count(1)
        .expect("shrink active membership");
    assert_state_error(
        state.revoke_member_permission(2, Permissions::REMOVE_FEED),
        StateError::UnauthorizedCaller,
    );
}

#[test]
fn get_member_permissions_returns_none_for_invalid_slot() {
    let state = governance_fixture(2);
    assert!(state.get_member_permissions(5).is_none());
}

#[test]
fn get_member_permissions_returns_some_for_active_member() {
    let state = governance_fixture(3);
    let perms = state.get_member_permissions(1).expect("active member");
    assert!(perms.as_u64() > 0);
}

#[test]
fn find_member_returns_index_and_permissions() {
    let state = governance_fixture(4);
    let member = active_member_slice(&state)[2];
    let (idx, perms) = state.find_member(&member).expect("member must be found");
    assert_eq!(idx, 2);
    assert_eq!(perms.as_u64(), active_permissions_slice(&state)[2].as_u64());
}

#[test]
fn find_member_excludes_inactive_slots() {
    let mut state = governance_fixture(4);
    let target = active_member_slice(&state)[3];
    state
        .set_active_member_count(2)
        .expect("shrink active membership");
    assert!(state.find_member(&target).is_none());
}

#[test]
fn check_member_permission_happy_path() {
    let state = governance_fixture(3);
    let member = active_member_slice(&state)[0];
    let perms = active_permissions_slice(&state)[0];

    // All fixtures include at least UPDATE_PRICE for member zero.
    assert!(perms.has_any(Permissions::UPDATE_PRICE));
    assert!(state
        .check_member_permission(&member, Permissions::UPDATE_PRICE)
        .is_ok());
}

#[test]
fn check_member_permission_errors_for_missing_capability() {
    let mut state = governance_fixture(2);
    let member = active_member_slice(&state)[0];

    state
        .revoke_member_permission(0, Permissions::ADD_FEED)
        .expect("clear add-feed capability");

    assert_state_error(
        state.check_member_permission(&member, Permissions::ADD_FEED),
        StateError::InsufficientPermissions,
    );
}

#[test]
fn check_member_permission_errors_for_unknown_identity() {
    let state = governance_fixture(3);
    let outsider = deterministic_pubkey(220);
    assert_state_error(
        state.check_member_permission(&outsider, Permissions::UPDATE_PRICE),
        StateError::UnauthorizedCaller,
    );
}

#[test]
fn allowed_program_counts_respect_capacity() {
    let state = governance_fixture(4);
    assert!(state.allowed_dex_program_count as usize <= MAX_ALLOWED_PROGRAMS);
    assert!(state.allowed_aggregator_program_count as usize <= MAX_ALLOWED_PROGRAMS);

    let dex_slice = &state.allowed_dex_programs[..state.allowed_dex_program_count as usize];
    assert!(dex_slice.iter().all(|key| *key != Pubkey::default()));

    let agg_slice =
        &state.allowed_aggregator_programs[..state.allowed_aggregator_program_count as usize];
    assert!(agg_slice.iter().all(|key| *key != Pubkey::default()));
}

#[test]
fn reserved_padding_remains_zero_after_mutations() {
    let mut state = governance_fixture(3);
    state
        .grant_member_permission(1, Permissions::TRIGGER_CIRCUIT_BREAKER)
        .expect("grant capability");
    state.allowed_dex_program_count =
        (state.allowed_dex_program_count + 1).min(MAX_ALLOWED_PROGRAMS as u8);

    assert_reserved_padding(&state);
}

#[test]
fn strict_mode_flag_is_configuration_only() {
    let mut state = governance_fixture(2);
    state.strict_mode_enabled = 1;
    let member = active_member_slice(&state)[0];
    assert!(state
        .check_member_permission(&member, Permissions::UPDATE_PRICE)
        .is_ok());
}
