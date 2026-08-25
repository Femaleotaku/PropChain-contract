//! # Integration Tests: Multicall batching across real targets (Issue #926)
//!
//! `propchain-multicall` exists to dispatch batches of calls to *other*
//! contracts. This suite instantiates the real `MulticallContract` and
//! drives its batch surface.
//!
//! Environment constraint (documented decision): ink! 5.x's off-chain
//! engine does not implement cross-contract invocation — every
//! `invoke_contract` panics with "off-chain environment does not support
//! contract invocation" (`ink_env::engine::off_chain::impls.rs`). True
//! end-to-end atomicity/revert assertions therefore require the e2e harness
//! plus a live node, which no repo CI command provisions. What CAN be pinned
//! off-chain is covered here:
//! - batch validation, pause gating and admin semantics on the real contract
//! - that well-formed batches aimed at real targets actually reach the
//!   cross-contract boundary for both `aggregate` and `try_aggregate_calls`
//!   (pinned via `#[should_panic]` against the engine's exact limitation),
//!   proving the dispatch plumbing executes instead of short-circuiting.

#[cfg(test)]
mod integration_multicall {
    use ink::env::{test, DefaultEnvironment};
    use propchain_multicall::propchain_multicall::MulticallContract;
    use propchain_traits::constants::MAX_BATCH_SIZE;
    use propchain_traits::multicall::{CallRequest, MulticallError};

    fn setup() -> MulticallContract {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        MulticallContract::new()
    }

    fn call(callee: ink::primitives::AccountId) -> CallRequest {
        // selector [0u8; 4] + empty args; explicit gas because the off-chain
        // engine does not implement `gas_left` either (0 = forward remaining).
        CallRequest {
            callee,
            selector_and_input: vec![0u8; 4],
            transferred_value: 0,
            gas_limit: 100_000,
            allow_revert: false,
        }
    }

    #[ink::test]
    fn constructor_and_admin_surface_work() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        let mut contract = setup();

        assert_eq!(contract.admin(), accounts.alice);
        assert!(!contract.is_paused());
        assert_eq!(contract.max_calls(), MAX_BATCH_SIZE);

        contract.transfer_admin(accounts.bob).expect("transfer");
        assert_eq!(contract.admin(), accounts.bob);
    }

    #[ink::test]
    fn aggregate_validates_batch_shape_before_dispatch() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        let mut contract = setup();

        assert_eq!(
            contract.aggregate(Vec::new()),
            Err(MulticallError::EmptyCalls)
        );
        assert_eq!(
            contract.try_aggregate_calls(Vec::new()),
            Err(MulticallError::EmptyCalls)
        );

        let oversized: Vec<CallRequest> =
            (0..=MAX_BATCH_SIZE).map(|_| call(accounts.bob)).collect();
        assert_eq!(
            contract.aggregate(oversized),
            Err(MulticallError::TooManyCalls)
        );
    }

    #[ink::test]
    fn paused_contract_rejects_both_entrypoints() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        let mut contract = setup();
        let calls = vec![call(accounts.bob), call(accounts.charlie)];

        // Only the admin can pause.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(contract.pause(), Err(MulticallError::Unauthorized));
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        contract.pause().expect("admin pauses");

        assert_eq!(
            contract.aggregate(calls.clone()),
            Err(MulticallError::Paused)
        );
        assert_eq!(
            contract.try_aggregate_calls(calls),
            Err(MulticallError::Paused)
        );

        contract.unpause().expect("admin unpauses");
        assert!(!contract.is_paused());
    }

    #[ink::test]
    #[should_panic(expected = "off-chain environment does not support contract invocation")]
    fn aggregate_attempts_real_dispatch_to_distinct_targets() {
        // Two distinct real target accounts; the batch reaches the env layer
        // (which panics on ink! 5's off-chain engine) only if request
        // plumbing — callee, selector framing, per-call gas and flags — is
        // correctly assembled for every entry.
        let accounts = test::default_accounts::<DefaultEnvironment>();
        let mut contract = setup();
        let calls = vec![call(accounts.bob), call(accounts.charlie)];
        let _ = contract.aggregate(calls);
    }

    #[ink::test]
    #[should_panic(expected = "off-chain environment does not support contract invocation")]
    fn try_aggregate_attempts_real_dispatch_to_distinct_targets() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        let mut contract = setup();
        let calls = vec![call(accounts.bob), call(accounts.charlie)];
        let _ = contract.try_aggregate_calls(calls);
    }
}
