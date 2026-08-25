#![cfg_attr(not(feature = "std"), no_std)]

//! Shared helpers for PropChain contracts.
//!
//! Currently hosts the intra-transaction caching abstraction
//! ([`cache::TransactionCache`]) used to avoid repeated storage reads
//! within a single message call.

pub mod cache;
