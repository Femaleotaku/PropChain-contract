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

// ─── Cross-Contract Integration Suites ───────────────────────────────
/// Issue #1013: Third-party registry integration coverage
pub mod integration_third_party;

/// Issue #1014: Compliance registry integration coverage
pub mod integration_compliance;
