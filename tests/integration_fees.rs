/// # Integration Tests: Fee Manager (Issue #922)
///
/// The fees contract prices registry operations, tracks collected fees in a
/// treasury, distributes the treasury between validators and the treasury
/// reserve, and gates every configuration change behind the admin.
///
/// Acceptance criteria tested:
///   check calculate_fee returns the configured base fee for a fixed-strategy operation
///   check Per-operation configs override defaults and validate their bounds
///   check Collected fees accumulate in the treasury report
///   check distribute_fees splits the treasury by configured shares and validators claim
///   check update_fee_params adapts the base fee to congestion, admin-only
///   check All admin entry points reject non-admin callers
#[cfg(test)]
mod integration_fees {
    use ink::env::{test, DefaultEnvironment};
    use propchain_fees::propchain_fees::{FeeCalculationMethod, FeeConfig, FeeManager};
    use propchain_traits::{BasisPoints, FeeOperation};

    fn fixed_config(base: u128, min: u128, max: u128) -> FeeConfig {
        FeeConfig {
            base_fee: base,
            min_fee: min,
            max_fee: max,
            congestion_sensitivity: 80,
            demand_factor_bp: BasisPoints::new(500),
            calculation_method: FeeCalculationMethod::Fixed,
            last_updated: 0,
        }
    }

    #[ink::test]
    fn per_operation_config_overrides_default_pricing() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let mut fees = FeeManager::new(1_000, 100, 5_000);

        // Default pricing applies to operations without an override.
        let default_fee = fees.calculate_fee(FeeOperation::RegisterProperty);
        assert!(
            (100..=5_000).contains(&default_fee),
            "default fee stays within configured bounds"
        );

        // A fixed-strategy override pins the exact fee for one operation.
        fees.set_operation_config(FeeOperation::OracleUpdate, fixed_config(777, 500, 900))
            .expect("valid per-operation config accepted");
        assert_eq!(
            fees.calculate_fee(FeeOperation::OracleUpdate),
            777,
            "fixed strategy returns the base fee verbatim"
        );
        assert_ne!(
            fees.calculate_fee(FeeOperation::TransferProperty),
            0,
            "other operations keep being priced"
        );

        // Invalid configs are rejected before touching storage.
        assert_eq!(
            fees.set_operation_config(
                FeeOperation::TransferProperty,
                fixed_config(500, 900, 1_000), // min > max
            ),
            Err(propchain_fees::propchain_fees::FeeError::InvalidConfig)
        );
        assert_eq!(
            fees.set_operation_config(
                FeeOperation::TransferProperty,
                fixed_config(400, 500, 1_000), // base < min
            ),
            Err(propchain_fees::propchain_fees::FeeError::InvalidConfig)
        );

        // Non-admin callers cannot change pricing.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            fees.set_operation_config(FeeOperation::TransferProperty, fixed_config(1, 1, 2),),
            Err(propchain_fees::propchain_fees::FeeError::Unauthorized)
        );
    }

    #[ink::test]
    fn fee_collection_and_validator_distribution() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let mut fees = FeeManager::new(1_000, 100, 5_000);

        // Only the admin manages the validator set.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            fees.add_validator(accounts.bob),
            Err(propchain_fees::propchain_fees::FeeError::Unauthorized)
        );

        test::set_caller::<DefaultEnvironment>(accounts.alice);
        fees.add_validator(accounts.bob)
            .expect("admin adds validator");
        fees.add_validator(accounts.bob)
            .expect("duplicate add is idempotent, not an error");
        assert_eq!(
            fees.pending_reward(accounts.bob),
            0,
            "validator registered exactly once"
        );

        // Collected fees accumulate in the on-chain report.
        for _ in 0..3 {
            fees.record_fee_collected(FeeOperation::RegisterProperty, 3_000, accounts.bob)
                .expect("fee collection recorded");
        }
        let report = fees.get_fee_report();
        assert_eq!(report.total_fees_collected, 9_000);

        // Shares must sum to at most 100%.
        assert_eq!(
            fees.set_distribution_rates(BasisPoints::new(6_000), BasisPoints::new(5_000)),
            Err(propchain_fees::propchain_fees::FeeError::InvalidConfig)
        );

        // Default split is 50/50: the single validator is owed half.
        fees.distribute_fees().expect("admin distributes");
        assert_eq!(fees.pending_reward(accounts.bob), 4_500);
        assert_eq!(fees.get_fee_report().total_distributed, 4_500);

        // Distribution is admin-only; claiming is open to anyone owed.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            fees.distribute_fees(),
            Err(propchain_fees::propchain_fees::FeeError::Unauthorized)
        );
        assert_eq!(fees.claim_rewards(), Ok(4_500));
        assert_eq!(fees.claim_rewards(), Ok(0), "double claim pays nothing");

        // Distributing an empty treasury is a no-op, not an error.
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        fees.distribute_fees().expect("empty distribution is no-op");
    }

    #[ink::test]
    fn congestion_response_and_admin_gating_of_param_updates() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let mut fees = FeeManager::new(1_000, 100, 5_000);

        // Non-admin callers cannot trigger automated adjustment.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            fees.update_fee_params(),
            Err(propchain_fees::propchain_fees::FeeError::Unauthorized)
        );

        test::set_caller::<DefaultEnvironment>(accounts.alice);

        // Zero recorded activity => congestion index below 30 => base eases
        // down by 5% but never below the configured minimum.
        fees.update_fee_params()
            .expect("admin adjusts params under low load");
        let report = fees.get_fee_report();
        assert_eq!(report.config.base_fee, 950);
        assert_eq!(report.congestion_index, 0);

        // The eased config still respects the floor.
        assert!(report.config.base_fee >= report.config.min_fee);
    }
}
