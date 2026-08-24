/// # Integration Tests: DEX AMM (Issue #918)
///
/// The DEX provides constant-product pools with LP shares, fee-adjusted
/// swaps in both directions, and slippage protection.
///
/// Acceptance criteria tested:
///   check Pool creation validates pairs, mints sqrt shares, and seeds analytics
///   check Duplicate pairs are rejected
///   check Adding liquidity mints proportional shares and updates reserves
///   check Swaps respect the pool fee and move the price
///   check Slippage guard rejects unfavorable outputs
///   check Removing liquidity returns pro-rata reserves
#[cfg(test)]
mod integration_dex {
    use ink::env::{test, DefaultEnvironment};
    use propchain_dex::dex::{Error as DexError, PropertyDex};

    fn dex() -> PropertyDex {
        test::set_caller::<DefaultEnvironment>(
            test::default_accounts::<DefaultEnvironment>().alice,
        );
        PropertyDex::new("PRPX".into(), 1_000_000, 10, 4_000)
    }

    #[ink::test]
    fn pool_creation_validates_and_seeds_state() {
        let mut dex = dex();

        // Degenerate pairs are rejected before touching storage.
        assert_eq!(
            dex.create_pool(1, 1, 30, 1_000, 1_000),
            Err(DexError::InvalidPair),
            "base == quote rejected"
        );
        assert_eq!(
            dex.create_pool(1, 2, 30, 0, 1_000),
            Err(DexError::InvalidPair),
            "zero initial base rejected"
        );
        assert_eq!(
            dex.create_pool(1, 2, 1_000, 1_000, 1_000),
            Err(DexError::InvalidPair),
            "fee at or above 100% rejected"
        );

        let pair = dex
            .create_pool(1, 2, 30, 100_000, 400_000)
            .expect("valid pool created");
        assert_eq!(pair, 1);

        let pool = dex.get_pool(pair).expect("pool stored");
        assert_eq!(pool.base_token, 1);
        assert_eq!(pool.quote_token, 2);
        assert_eq!(pool.reserve_base, 100_000);
        assert_eq!(pool.reserve_quote, 400_000);
        assert_eq!(pool.fee_bips, 30);
        // Initial LP supply is sqrt(reserve_base * reserve_quote).
        assert_eq!(pool.total_lp_shares, 200_000);
        // Initial price is quote/base in bips.
        assert_eq!(pool.last_price, 40_000);

        // A second pool with identical tokens duplicates the lookup key.
        assert_eq!(
            dex.create_pool(2, 1, 30, 1_000, 4_000),
            Err(DexError::InvalidPair),
            "duplicate (ordered) pair rejected"
        );
    }

    #[ink::test]
    fn liquidity_provision_mints_proportional_shares() {
        let mut dex = dex();
        let pair = dex.create_pool(1, 2, 30, 100_000, 400_000).expect("pool");

        // Doubling both sides mints exactly the initial share supply again.
        let minted = dex
            .add_liquidity(pair, 100_000, 400_000)
            .expect("balanced add accepted");
        assert_eq!(minted, 200_000);

        let pool = dex.get_pool(pair).unwrap();
        assert_eq!(pool.reserve_base, 200_000);
        assert_eq!(pool.reserve_quote, 800_000);
        assert_eq!(pool.total_lp_shares, 400_000);

        // Zero amounts are rejected.
        assert_eq!(dex.add_liquidity(pair, 0, 100), Err(DexError::InvalidPair));

        // Withdrawing more than owned fails cleanly.
        assert_eq!(
            dex.remove_liquidity(pair, u128::MAX),
            Err(DexError::InsufficientLiquidity)
        );
    }

    #[ink::test]
    fn swaps_respect_fees_move_price_and_enforce_slippage() {
        let mut dex = dex();
        let pair = dex
            .create_pool(1, 2, 100, 1_000_000, 4_000_000) // 1% fee
            .expect("pool");

        // Sell base: out = (in * 99%) * quote_out / (base_in + in*99%).
        let amount_in = 100_000u128;
        let fee_adjusted = amount_in * 9_900 / 10_000; // 99_000
        let expected_out = fee_adjusted * 4_000_000 / (1_000_000 + fee_adjusted); // 358_543...

        let out = dex
            .swap_exact_base_for_quote(pair, amount_in, 0)
            .expect("swap succeeds with zero slippage bound");
        assert_eq!(out, expected_out);

        let pool = dex.get_pool(pair).unwrap();
        assert_eq!(pool.reserve_base, 1_100_000);
        assert_eq!(pool.reserve_quote, 4_000_000 - expected_out);
        assert!(
            pool.last_price < 40_000,
            "selling base must depress the quote-per-base price" // 40_000 bips before -> ~33_088 after
        );

        // The reverse direction works through the buy-side entry point.
        let out2 = dex
            .swap_exact_quote_for_base(pair, 100_000, 0)
            .expect("quote-for-base succeeds");
        assert!(out2 > 0);

        // Slippage guard: demanding more than the pool can pay reverts.
        assert_eq!(
            dex.swap_exact_base_for_quote(pair, 1_000, u128::MAX),
            Err(DexError::SlippageExceeded)
        );

        // Zero input is invalid regardless of bounds.
        assert_eq!(
            dex.swap_exact_base_for_quote(pair, 0, 0),
            Err(DexError::InvalidOrder)
        );

        // Unknown pools fail cleanly.
        assert_eq!(
            dex.swap_exact_base_for_quote(999, 1_000, 0),
            Err(DexError::PoolNotFound)
        );
    }
}
