#![no_std]

use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, token::Client as TokenClient, Address,
    Bytes, Env,
};

#[contractclient(name = "AmmPoolClient")]
pub trait AmmPool {
    fn get_info(env: Env) -> PoolInfo;
}

#[contracttype]
pub struct PoolInfo {
    pub token_a: Address,
    pub token_b: Address,
    pub reserve_a: i128,
    pub reserve_b: i128,
    pub total_shares: i128,
    pub fee_bps: i128,
    pub flash_loan_fee_bps: i128,
    pub admin: Address,
    pub fee_recipient: Address,
    pub protocol_fee_bps: i128,
    pub lp_rebate_bps: i128,
}

#[contracttype]
enum DataKey {
    Pool,
}

#[contract]
pub struct ExampleFlashLoanReceiver;

#[contractimpl]
impl ExampleFlashLoanReceiver {
    pub fn initialize(env: Env, pool: Address) {
        env.storage().instance().set(&DataKey::Pool, &pool);
    }

    pub fn on_flash_loan(
        env: Env,
        token_a_amount: i128,
        token_b_amount: i128,
        fee_a: i128,
        fee_b: i128,
        _data: Bytes,
    ) -> bool {
        let pool: Address = env.storage().instance().get(&DataKey::Pool).unwrap();
        let info = AmmPoolClient::new(&env, &pool).get_info();
        let receiver = env.current_contract_address();

        if token_a_amount > 0 || fee_a > 0 {
            TokenClient::new(&env, &info.token_a).transfer(
                &receiver,
                &pool,
                &(token_a_amount + fee_a),
            );
        }

        if token_b_amount > 0 || fee_b > 0 {
            TokenClient::new(&env, &info.token_b).transfer(
                &receiver,
                &pool,
                &(token_b_amount + fee_b),
            );
        }

        true
    }
}
