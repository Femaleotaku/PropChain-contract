//! PropChain Test Suite
//!
//! This module provides the test library for PropChain contracts,
//! including shared utilities, fixtures, and test helpers.

#![cfg_attr(not(feature = "std"), no_std)]

// Core test modules
pub mod bridge_load_tests;
pub mod test_utils; // Load testing framework

// Re-export commonly used items
pub use test_utils::*;

// ─── Security Test Modules ───────────────────────────────────────────
pub mod security_audit_runner;

// ─── Regression Test Suite ───────────────────────────────────────────
/// Issue #487: Regression test suite for all previously fixed bugs
pub mod regression;

// ─── Integration Test Modules ────────────────────────────────────────
/// Issue #1001: Insurance integration coverage (policy lifecycle, claims,
/// admin/oracle authorization paths)
pub mod integration_insurance;

/// Issue #1002: Governance integration coverage (signers → proposal →
/// votes → timelock → execution)
pub mod integration_governance;

/// Issues #1003 / #1004: Monitoring and sanctions screening integration
/// coverage (admin surface, pause gating; sanctioned entity/property flows)
pub mod integration_monitoring_sanctions;
