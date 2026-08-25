/// # Integration Tests: Lending Lifecycle (Issue #919)
///
/// The lending contract prices pools by utilization, gates loans behind
/// on-chain credit profiles, and liquidates when collateral value collapses.
///
/// Acceptance criteria tested:
///   check Pool creation is admin-gated and borrow rates track utilization
///   check Deposit/borrow accounting rejects over-borrowing
///   check Loans require a qualifying credit profile before underwriting
///   check Underwriting enforces LTV limits and activates qualified loans
///   check Liquidation triggers only past the threshold and settles state
#[cfg(test)]
mod integration_lending {
    use ink::env::{test, DefaultEnvironment};
    use propchain_lending::propchain_lending::{LoanStatus, PropertyLending};

    /// Admin = alice; bob gets a 620 credit profile (500 base + 6 * 20).
    fn funded_lending() -> (PropertyLending, u64) {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let mut lending = PropertyLending::new(accounts.alice);
        for _ in 0..6 {
            lending
                .record_repayment(accounts.bob)
                .expect("admin records repayment");
        }
        (lending, 1)
    }

    #[ink::test]
    fn pool_rates_track_utilization() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        let (mut lending, _) = funded_lending();

        // Pool creation is admin-only.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            lending.create_pool(200),
            Err(propchain_lending::propchain_lending::LendingError::Unauthorized)
        );

        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let pool_id = lending.create_pool(200).expect("admin creates pool");

        // Empty utilization: rate == base rate.
        assert_eq!(lending.borrow_rate(pool_id), Ok(200));

        // Deposits then borrows push the rate up with utilization.
        lending.deposit(pool_id, 10_000).expect("deposit accepted");
        assert_eq!(
            lending.borrow(pool_id, 20_000),
            Err(propchain_lending::propchain_lending::LendingError::InsufficientLiquidity),
            "cannot borrow beyond deposits"
        );
        lending.borrow(pool_id, 5_000).expect("half utilized");
        // utilisation = 50% -> base + (5000/50) = 200 + 100.
        assert_eq!(lending.borrow_rate(pool_id), Ok(300));

        // Unknown pools fail cleanly.
        assert_eq!(
            lending.borrow_rate(999),
            Err(propchain_lending::propchain_lending::LendingError::PoolNotFound)
        );
    }

    #[ink::test]
    fn underwriting_gates_on_credit_and_ltv() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        let (mut lending, _) = funded_lending();

        test::set_block_timestamp::<DefaultEnvironment>(100);

        // Collateral assessed by admin for property 10.
        lending
            .assess_collateral(10, 1_000_000, 7_000, 8_000)
            .expect("admin assesses collateral");

        // Bob (score 620) applies within LTV limits: 400k / 1M = 4000 bps.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        let loan_id = lending
            .apply_for_loan(10, 400_000, 1_000_000, 999)
            .expect("application stored");

        // Pending until an admin underwrites...
        assert_eq!(
            lending.get_loan(loan_id).unwrap().status,
            LoanStatus::Pending
        );
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        assert!(lending.underwrite_loan(loan_id).expect("underwritten"));

        let loan = lending.get_loan(loan_id).unwrap();
        assert_eq!(loan.status, LoanStatus::Active);
        assert_eq!(loan.credit_score, 620);

        // A second borrower with no repayment history fails underwriting.
        test::set_caller::<DefaultEnvironment>(accounts.charlie);
        let weak = lending
            .apply_for_loan(10, 300_000, 1_000_000, 900)
            .expect("second application stored");
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        assert!(
            !lending.underwrite_loan(weak).unwrap(),
            "thin file rejected"
        );
        assert_eq!(
            lending.get_loan(weak).unwrap().status,
            LoanStatus::Pending,
            "rejected application stays pending"
        );
    }

    #[ink::test]
    fn liquidation_settles_only_past_threshold() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        let (mut lending, _) = funded_lending();

        test::set_block_timestamp::<DefaultEnvironment>(100);
        lending
            .assess_collateral(10, 1_000_000, 7_000, 8_000)
            .expect("collateral assessed");

        test::set_caller::<DefaultEnvironment>(accounts.bob);
        let loan_id = lending
            .apply_for_loan(10, 500_000, 1_000_000, 999)
            .expect("loan applied");

        test::set_caller::<DefaultEnvironment>(accounts.alice);
        assert!(lending.underwrite_loan(loan_id).unwrap());

        // Healthy market: no liquidation signal, message refuses to act.
        assert_eq!(
            lending.should_liquidate_loan(loan_id, vec![(10, 2_000_000)]),
            Ok(false)
        );
        assert_eq!(
            lending.liquidate_loan(loan_id, vec![(10, 2_000_000)]),
            Err(propchain_lending::propchain_lending::LendingError::LiquidationThresholdNotMet)
        );

        // Collateral halves twice: debt/value crosses the 8000 bps threshold.
        assert_eq!(
            lending.should_liquidate_loan(loan_id, vec![(10, 550_000)]),
            Ok(true)
        );
        lending
            .liquidate_loan(loan_id, vec![(10, 550_000)])
            .expect("breached loan liquidated");

        // Settled state cannot be re-liquidated.
        assert_eq!(
            lending.get_loan(loan_id).unwrap().status,
            LoanStatus::Liquidated
        );
        assert_eq!(
            lending.liquidate_loan(loan_id, vec![(10, 550_000)]),
            Err(propchain_lending::propchain_lending::LendingError::LoanNotActive)
        );
        assert_eq!(
            lending.should_liquidate_loan(loan_id, vec![(10, 100_000)]),
            Ok(false),
            "settled loans short-circuit the risk check"
        );
    }
}
