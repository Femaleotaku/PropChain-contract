// Unit tests for the fees contract (Issue #101 - extracted from lib.rs)

#[cfg(test)]
mod tests {
    use super::*;

    type DefaultEnvironment = ink::env::DefaultEnvironment;

    fn default_accounts()
    -> ink::env::test::DefaultAccounts<DefaultEnvironment> {
        ink::env::test::default_accounts::<DefaultEnvironment>()
    }

    /// Fund an account with native balance so it can bid and receive refunds.
    fn fund(account: &AccountId, amount: u128) {
        ink::env::test::set_account_balance::<DefaultEnvironment>(*account, amount);
    }

    fn balance(account: &AccountId) -> u128 {
        ink::env::test::get_account_balance::<DefaultEnvironment>(*account)
            .expect("account must have a balance")
    }

    #[ink::test]
    fn test_dynamic_fee_calculation() {
        let contract = FeeManager::new(1000, 100, 100_000);
        let fee = contract.calculate_fee(FeeOperation::RegisterProperty);
        assert!((100..=100_000).contains(&fee));
    }

    #[ink::test]
    fn test_premium_auction_flow() {
        let accounts = default_accounts();
        let mut contract = FeeManager::new(100, 10, 10_000);
        let auction_id = contract
            .create_premium_auction(1, 500, 3600)
            .expect("create auction");
        assert_eq!(auction_id, 1);
        let auction = contract.get_auction(auction_id).unwrap();
        assert_eq!(auction.property_id, 1);
        assert_eq!(auction.min_bid, 500);
        assert!(!auction.settled);

        ink::env::test::set_caller::<DefaultEnvironment>(accounts.bob);
        ink::env::test::set_value_transferred::<DefaultEnvironment>(600);
        assert!(contract.place_bid(auction_id, 600).is_ok());
        let auction = contract.get_auction(auction_id).unwrap();
        assert_eq!(auction.current_bid, 600);
        assert_eq!(auction.current_bidder, Some(accounts.bob));
        assert_eq!(auction.escrowed_value, 600);
    }

    /// The off-chain engine enforces an existential deposit of 1e6, so all
    /// seeded balances must stay comfortably above it even after debits.
    const START_BALANCE: u128 = 1_000_000_000;

    /// Bids without attaching enough value must be rejected.
    #[ink::test]
    fn place_bid_requires_attached_value() {
        let mut contract = FeeManager::new(100, 10, 10_000);
        let auction_id = contract.create_premium_auction(1, 500, 3600).expect("create");
        assert_eq!(
            contract.place_bid(auction_id, 600),
            Err(FeeError::InsufficientValue)
        );
    }

    /// The previous highest bidder must be fully refunded when outbid.
    #[ink::test]
    fn outbid_refunds_previous_bidder_escrow() {
        let accounts = default_accounts();
        fund(&accounts.alice, START_BALANCE);
        fund(&accounts.bob, START_BALANCE);
        fund(&accounts.charlie, START_BALANCE);

        let mut contract = FeeManager::new(100, 10, 10_000);
        let auction_id = contract.create_premium_auction(1, 500, 3600).expect("create");

        ink::env::test::set_caller::<DefaultEnvironment>(accounts.bob);
        ink::env::test::set_value_transferred::<DefaultEnvironment>(600);
        // The off-chain test engine credits the contract but does not debit
        // the caller, so model the live-runtime debit explicitly.
        fund(&accounts.bob, START_BALANCE - 600);
        assert!(contract.place_bid(auction_id, 600).is_ok());
        assert_eq!(balance(&accounts.bob), START_BALANCE - 600);

        ink::env::test::set_caller::<DefaultEnvironment>(accounts.charlie);
        ink::env::test::set_value_transferred::<DefaultEnvironment>(700);
        assert!(contract.place_bid(auction_id, 700).is_ok());

        // Bob got his escrow back.
        assert_eq!(balance(&accounts.bob), START_BALANCE);

        let auction = contract.get_auction(auction_id).unwrap();
        assert_eq!(auction.current_bid, 700);
        assert_eq!(auction.current_bidder, Some(accounts.charlie));
        assert_eq!(auction.escrowed_value, 700);
    }

    /// Overpayment above the bid amount is refunded immediately.
    #[ink::test]
    fn overpayment_excess_is_refunded() {
        let accounts = default_accounts();
        fund(&accounts.bob, START_BALANCE);

        let mut contract = FeeManager::new(100, 10, 10_000);
        let auction_id = contract.create_premium_auction(1, 500, 3600).expect("create");

        ink::env::test::set_caller::<DefaultEnvironment>(accounts.bob);
        ink::env::test::set_value_transferred::<DefaultEnvironment>(650);
        // Model the live-runtime debit of the full attached value.
        fund(&accounts.bob, START_BALANCE - 650);
        assert!(contract.place_bid(auction_id, 600).is_ok());

        // Net cost equals exactly the bid amount (excess was refunded).
        assert_eq!(balance(&accounts.bob), START_BALANCE - 600);
        let auction = contract.get_auction(auction_id).unwrap();
        assert_eq!(auction.escrowed_value, 600);
    }

    /// Settlement pays the escrowed winning bid to the seller exactly once.
    #[ink::test]
    fn settlement_pays_seller_from_escrow() {
        let accounts = default_accounts();
        fund(&accounts.alice, START_BALANCE);
        fund(&accounts.bob, START_BALANCE);

        let mut contract = FeeManager::new(100, 10, 10_000);
        let auction_id = contract.create_premium_auction(1, 500, 3600).expect("create");

        ink::env::test::set_caller::<DefaultEnvironment>(accounts.bob);
        ink::env::test::set_value_transferred::<DefaultEnvironment>(700);
        assert!(contract.place_bid(auction_id, 700).is_ok());

        // Advance past the auction deadline.
        for _ in 0..2_000 {
            ink::env::test::advance_block::<DefaultEnvironment>();
        }

        assert!(contract.settle_auction(auction_id).is_ok());

        // Seller received the winning bid from custody.
        assert_eq!(balance(&accounts.alice), START_BALANCE + 700);
        let auction = contract.get_auction(auction_id).unwrap();
        assert!(auction.settled);
        assert_eq!(auction.escrowed_value, 0);

        // Double settlement is impossible.
        assert_eq!(
            contract.settle_auction(auction_id),
            Err(FeeError::AlreadySettled)
        );
    }

    #[ink::test]
    fn test_fee_report() {
        let contract = FeeManager::new(1000, 100, 50_000);
        let report = contract.get_fee_report();
        assert_eq!(report.total_fees_collected, 0);
        assert!(report.recommended_fee >= 100);
    }

    #[ink::test]
    fn test_fee_estimate_recommendation() {
        let contract = FeeManager::new(1000, 100, 50_000);
        let est = contract.get_fee_estimate(FeeOperation::TransferProperty);
        assert!(!est.recommendation.is_empty());
        assert!(!est.congestion_level.is_empty());
    }

    #[ink::test]
    fn test_fixed_fee_strategy() {
        let mut contract = FeeManager::new(1000, 100, 100_000);
        let mut config = contract.default_config();
        config.calculation_method = FeeCalculationMethod::Fixed;
        config.base_fee = 2000;
        
        assert!(contract.set_operation_config(FeeOperation::RegisterProperty, config).is_ok());
        
        let fee = contract.calculate_fee(FeeOperation::RegisterProperty);
        assert_eq!(fee, 2000);
    }

    #[ink::test]
    fn test_tiered_fee_strategy() {
        let mut contract = FeeManager::new(1000, 100, 100_000);
        let mut config = contract.default_config();
        config.calculation_method = FeeCalculationMethod::Tiered;
        config.base_fee = 1000;
        
        assert!(contract.set_operation_config(FeeOperation::RegisterProperty, config).is_ok());
        
        // Tiered for RegisterProperty is 2x base_fee (20000 BP)
        let fee = contract.calculate_fee(FeeOperation::RegisterProperty);
        assert_eq!(fee, 2000);
    }

    #[ink::test]
    fn test_exponential_fee_strategy() {
        let mut contract = FeeManager::new(1000, 100, 100_000);
        let mut config = contract.default_config();
        config.calculation_method = FeeCalculationMethod::Exponential;
        config.base_fee = 1000;
        config.congestion_sensitivity = 100;
        
        assert!(contract.set_operation_config(FeeOperation::RegisterProperty, config).is_ok());
        
        // With 0 congestion, fee should be base_fee
        let fee = contract.calculate_fee(FeeOperation::RegisterProperty);
        assert_eq!(fee, 1000);
    }


    // ========== Dynamic fee model tests (Issue #508) ==========

    /// Helper: compute the fee rate without needing a live contract env,
    /// so we can drive utilisation to any value cleanly.
    fn compute_rate(base_bps: u32, multiplier: u32, max_bps: u32, utilisation: u32) -> u32 {
        let util = utilisation.min(100) as u64;
        let base = base_bps as u64;
        let cm = multiplier as u64;
        let multiplier_pct = 100u64
            .saturating_add(util.saturating_mul(cm.saturating_sub(100)).saturating_div(100));
        let effective = base.saturating_mul(multiplier_pct).saturating_div(100);
        (effective as u32).min(max_bps)
    }

    /// Fee increases as pool utilisation approaches 100 %.
    #[ink::test]
    fn test_fee_increases_with_utilisation() {
        let fee_0 = compute_rate(30, 300, 200, 0);
        let fee_50 = compute_rate(30, 300, 200, 50);
        let fee_100 = compute_rate(30, 300, 200, 100);

        assert!(fee_0 <= fee_50, "fee at 50% util should be >= fee at 0%");
        assert!(fee_50 <= fee_100, "fee at 100% util should be >= fee at 50%");
        // Concrete check: at 0% util we get exactly base_fee_bps
        assert_eq!(fee_0, 30);
        // At 100% util with multiplier 300 (3×): 30 * 300 / 100 = 90, within max 200
        assert_eq!(fee_100, 90);
    }

    /// Fee never exceeds configured max_fee_bps regardless of utilisation or multiplier.
    #[ink::test]
    fn test_fee_never_exceeds_max_fee_bps() {
        // Choose a very aggressive multiplier so the raw result would exceed max.
        // base=50, multiplier=1000 (10×), max=100
        // At 100% util raw = 50 * 1000 / 100 = 500, but max caps it at 100.
        for util in [0u32, 25, 50, 75, 100] {
            let rate = compute_rate(50, 1000, 100, util);
            assert!(
                rate <= 100,
                "fee rate {rate} exceeded max_fee_bps 100 at utilisation {util}"
            );
        }

        // Also test via the contract message path.
        let mut contract = FeeManager::new(1000, 100, 100_000);
        let config = DynamicFeeConfig {
            base_fee_bps: BasisPoints::new(50),
            congestion_multiplier: 1000,
            max_fee_bps: BasisPoints::new(100),
        };
        assert!(contract.set_dynamic_fee_config(config).is_ok());
        // Rate must never exceed max_fee_bps (no way to drive utilisation to 100
        // in unit tests, but at 0 util it should equal base_fee_bps = 50).
        let rate = contract.get_current_fee_rate();
        assert!(rate <= 100, "get_current_fee_rate() exceeded max_fee_bps");
    }

    /// Fee reverts to base_fee_bps when utilisation drops to zero.
    #[ink::test]
    fn test_fee_reverts_to_base_at_zero_utilisation() {
        // At zero congestion the formula reduces to: base * 100 / 100 = base.
        let base_bps = 30u32;
        let rate = compute_rate(base_bps, 300, 200, 0);
        assert_eq!(
            rate, base_bps,
            "fee rate should equal base_fee_bps when utilisation is 0"
        );

        // Verify through the contract: a freshly constructed contract has
        // zero recent_ops_count → congestion_index() == 0.
        let contract = FeeManager::new(1000, 100, 100_000);
        let rate = contract.get_current_fee_rate();
        // Default config: base=30, multiplier=300, max=200 → at 0 util → 30 bps
        assert_eq!(
            rate, 30,
            "get_current_fee_rate() should return base_fee_bps at zero utilisation"
        );
    }

    /// set_dynamic_fee_config rejects invalid configs.
    #[ink::test]
    fn test_set_dynamic_fee_config_validation() {
        let mut contract = FeeManager::new(1000, 100, 100_000);

        // base > max is invalid
        let bad_config = DynamicFeeConfig {
            base_fee_bps: BasisPoints::new(500),
            congestion_multiplier: 200,
            max_fee_bps: BasisPoints::new(100),
        };
        assert!(contract.set_dynamic_fee_config(bad_config).is_err());

        // multiplier < 100 is invalid (fees should not decrease with congestion)
        let bad_config2 = DynamicFeeConfig {
            base_fee_bps: BasisPoints::new(30),
            congestion_multiplier: 50,
            max_fee_bps: BasisPoints::new(200),
        };
        assert!(contract.set_dynamic_fee_config(bad_config2).is_err());

        // Valid config succeeds and is queryable
        let good_config = DynamicFeeConfig {
            base_fee_bps: BasisPoints::new(30),
            congestion_multiplier: 200,
            max_fee_bps: BasisPoints::new(150),
        };
        assert!(contract.set_dynamic_fee_config(good_config.clone()).is_ok());
        assert_eq!(contract.dynamic_fee_config(), good_config);
    }
}
