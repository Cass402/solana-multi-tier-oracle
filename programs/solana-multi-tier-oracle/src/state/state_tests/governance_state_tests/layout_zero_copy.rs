//! Byte-level ABI tests for `GovernanceState` and `Permissions`.
//!
//! Why these tests matter
//! - The program uses a zero-copy account model: on-chain accounts are mapped
//!   directly to Rust structs and read/written without intermediate
//!   serialization. This gives significant runtime and compute benefits on
//!   Solana, but imposes strict requirements on struct layout, field
//!   alignment, reserved padding, and ABI stability.
//!
//! Design & safety considerations
//! - Any change in struct size, field order, alignment, or padding can silently
//!   break deserialization assumptions and corrupt on-chain data. Tests in this
//!   file encode those layout invariants and exist to force a migration plan
//!   before any on-chain upgrade if they fail.
//! - We intentionally assert exact sizes and alignments, validate ZeroCopy/Pod
//!   contracts, and verify that reserved padding remains zero after mutations.
//!   These checks help auditors and developers reason about forward-compatibility
//!   and the safety of in-place memory manipulations.

use super::helpers::{
    assert_permissions_sanitized, assert_reserved_padding, governance_fixture, governance_to_bytes,
};
use crate::state::governance_state::{GovernanceState, Permissions};
use crate::utils::constants::MAX_ALLOWED_PROGRAMS;
use anchor_lang::Space;
use bytemuck::{bytes_of, Pod, Zeroable};
use std::mem::{align_of, size_of};

#[test]
fn governance_state_layout_contract() {
    const EXPECTED_SIZE: usize = 1_744;
    assert_eq!(
        size_of::<GovernanceState>(),
        EXPECTED_SIZE,
        "GovernanceState size drifted; a layout change requires an explicit migration strategy before mainnet deployment"
    );
    assert_eq!(
        align_of::<GovernanceState>(),
        8,
        "GovernanceState alignment must remain 8 bytes to preserve u64 field alignment and zero-copy safety"
    );

    assert_eq!(
        size_of::<Permissions>(),
        8,
        "Permissions must remain a compact u64 wrapper to allow bitwise operations and compact storage"
    );
    assert_eq!(
        align_of::<Permissions>(),
        8,
        "Permissions alignment is part of the zero-copy ABI contract and must remain stable"
    );
}

#[test]
fn init_space_matches_struct_layout() {
    const DISCRIMINATOR: usize = 8;
    assert_eq!(
        GovernanceState::INIT_SPACE,
        size_of::<GovernanceState>(),
        "INIT_SPACE must mirror the raw struct size so Anchor account allocation matches the in-memory layout"
    );
    assert_eq!(
        DISCRIMINATOR + GovernanceState::INIT_SPACE,
        DISCRIMINATOR + size_of::<GovernanceState>(),
        "Anchor account sizing (discriminator + payload) must remain stable; changing this affects rents and account creation expectations"
    );
}

#[test]
fn zero_copy_trait_contracts_hold() {
    fn assert_pod<T: Pod>() {}
    fn assert_zeroable<T: Zeroable>() {}
    assert_pod::<Permissions>();
    assert_zeroable::<Permissions>();
    assert_zeroable::<GovernanceState>();

    // GovernanceState implements Anchor's ZeroCopy contract; enforce Pod/Zeroable
    // via runtime assertion using bytes_of to prove the implementation works.
    //
    // `bytes_of` produces a byte slice referencing the struct. This proves that
    // the type has a stable memory representation compatible with `Pod`/`Zeroable`.
    let fixture = governance_fixture(4);
    assert!(bytes_of(&fixture).len() == size_of::<GovernanceState>());
}

#[test]
fn padding_bytes_remain_zero_after_mutations() {
    let mut state = governance_fixture(5);
    state
        .grant_member_permission(0, Permissions::EMERGENCY_HALT)
        .expect("grant capability");
    state.allowed_dex_program_count =
        (state.allowed_dex_program_count + 1).min(MAX_ALLOWED_PROGRAMS as u8);

    // Confirm in-memory reserved padding stays zeroed. If this assertion
    // fails it usually means a layout or write-overrun bug that could corrupt
    // future fields when persisted on-chain.
    assert_reserved_padding(&state);

    let bytes = governance_to_bytes(&state);
    let padding_slice = &bytes[bytes.len() - state.reserved.len()..];
    assert!(
        padding_slice.iter().all(|byte| *byte == 0),
        "serialized image leaked non-zero padding; serialization must not include uninitialized or stale bytes"
    );
}

#[test]
fn zero_copy_load_roundtrip_matches_fixture() {
    let state = governance_fixture(3);
    let mut account_image = governance_to_bytes(&state);
    let view = unsafe {
        // Safety: the byte buffer originated from a GovernanceState produced by
        // `governance_to_bytes` earlier in the test. We assert that the buffer
        // length matches the struct size and that the fixture was produced by a
        // properly initialized instance. Casting bytes -> struct is unsafe and
        // only acceptable in this controlled test environment to validate
        // zero-copy roundtrips. Never perform this cast on untrusted input.
        &mut *(account_image.as_mut_ptr() as *mut GovernanceState)
    };
    assert_eq!(view.proposal_threshold, state.proposal_threshold);
    assert_eq!(view.voting_period, state.voting_period);
    assert_eq!(view.execution_delay, state.execution_delay);
    assert_eq!(view.timelock_duration, state.timelock_duration);
    assert_eq!(view.veto_period, state.veto_period);
    assert_eq!(view.quorum_threshold, state.quorum_threshold);
    assert_eq!(view.multi_sig_threshold, state.multi_sig_threshold);
    assert_eq!(view.active_member_count, state.active_member_count);
    assert_eq!(view.oracle_state, state.oracle_state);

    for (view_perm, source_perm) in view
        .member_permissions
        .iter()
        .zip(state.member_permissions.iter())
        .take(state.active_member_count as usize)
    {
        assert_permissions_sanitized(*view_perm);
        assert_eq!(view_perm.as_u64(), source_perm.as_u64());
    }
}
