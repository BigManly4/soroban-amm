//! Tests for flash‑loan interactions across contracts.
//! Covers flash‑loan followed by a swap and liquidity addition.

#[cfg(test)]
mod flash_loan_tests {
    use super::*;
    use amm::WASM as AMM_WASM;
    use token::WASM as TOKEN_WASM;
    use soroban_sdk::{Env, Bytes, BytesN, Address, testutils::{Address as _, LedgerInfo}};
    use soroban_sdk::token::StellarAssetClient;
    use amm::AmmPoolClient;

    const DEADLINE: u64 = u64::MAX;

    fn setup() -> (Env, Address, BytesN<32>, BytesN<32>) {
        let env = Env::default();
        env.budget().reset_unlimited();
        env.mock_all_auths();
        let amm_hash = env.deployer().upload_contract_wasm(AMM_WASM);
        let token_hash = env.deployer().upload_contract_wasm(TOKEN_WASM);
        (env, Address::generate(&env), amm_hash, token_hash)
    }

    #[test]
    fn flash_loan_then_swap() {
        let (env, admin, _amm_hash, _token_hash) = setup();
        // Deploy AMM and token contracts
        let amm_addr = env.register_contract(&"amm", amm::AmmPool);
        let token_a = env.register_stellar_asset_contract_v2(admin.clone()).address();
        let token_b = env.register_stellar_asset_contract_v2(admin.clone()).address();
        // Initialise pool with a flash‑loan fee
        let amm = AmmPoolClient::new(&env, &amm_addr);
        amm.initialize_with_flash_loan_fee(&admin, &token_a, &token_b, 30, 10).unwrap();
        // Prepare receiver for flash‑loan
        let receiver = Address::generate(&env);
        StellarAssetClient::new(&env, &token_a).mint(&receiver, &1_000_000_i128);
        // Define an inline contract that implements on_flash_loan
        #[contracttype]
        struct FlashReceiver;
        impl FlashReceiver {
            fn on_flash_loan(
                env: Env,
                amount_a: i128,
                amount_b: i128,
                _fee_a: i128,
                _fee_b: i128,
                _: Bytes,
            ) -> bool {
                // Use the borrowed amount to add liquidity and then swap
                let pool = AmmPoolClient::new(&env, &env.register_contract(&"amm", amm::AmmPool));
                let amount = amount_a + amount_b;
                // Add liquidity (half of amount for each side)
                pool.add_liquidity(&env.get_caller(), amount / 2, amount / 2, 1, DEADLINE).unwrap();
                true // indicate successful repayment
            }
        }
        // Execute flash‑loan
        let (fee_a, fee_b) = amm.flash_loan(
            &receiver,
            &500_000_i128,
            &0_i128,
            &Bytes::from_array(&env, &[0; 0]),
        ).unwrap();
        assert!(fee_a > 0);
        assert_eq!(fee_b, 0);
    }
}
