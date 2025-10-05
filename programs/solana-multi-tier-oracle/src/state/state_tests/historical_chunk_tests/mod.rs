//! Test harness for `HistoricalChunk` circular buffer invariants and integration behaviour.
//!
//! The module is split into focused submodules to mirror the AUDIT checklist:
//! - `core_unit_tests`: deterministic unit coverage of push/latest/has_next primitives.
//! - `layout_zero_copy`: byte-level layout + zero-copy trait contracts.
//! - `property_tests`: proptest-powered fuzzing of FIFO invariants under randomized input.
//! - `serialization_and_integration`: serialization round-trips and OracleState coupling.
//! - `helpers`: shared fixtures, builders, and invariant assertions used across suites.
//!
//! Keeping the modules granular clarifies intent for auditors and makes it easy to
//! extend coverage as new invariants are introduced.

pub mod core_unit_tests;
pub mod helpers;
pub mod instruction_integration;
pub mod layout_zero_copy;
pub mod property_tests;
pub mod serialization_and_integration;
