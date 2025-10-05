//! Comprehensive test harness for governance account invariants.
//!
//! The governance subsystem protects production upgrade keys and operational
//! controls. These tests are organised into focused modules so auditors can
//! reason about coverage:
//! - `helpers`: deterministic fixtures shared across suites.
//! - `permissions_unit`: atomic permission bit manipulation behaviour.
//! - `core_unit_tests`: GovernanceState member-management primitives.
//! - `layout_zero_copy`: byte-level ABI and zero-copy guarantees.
//! - `property_tests`: proptest-based fuzzing of permission masks.
//! - `serialization_and_integration`: round-trips plus OracleState coupling.
//! - `attack_scenarios`: regression harness for common governance threats.
//! - `timing_and_thresholds`: boundary validation for proposal timing knobs.
//! - `stress_sequences`: rapid update simulations mirroring operator churn.

pub mod attack_scenarios;
pub mod core_unit_tests;
pub mod helpers;
pub mod layout_zero_copy;
pub mod permissions_unit;
pub mod property_tests;
pub mod serialization_and_integration;
pub mod stress_sequences;
pub mod timing_and_thresholds;
