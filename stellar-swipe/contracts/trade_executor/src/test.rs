#![cfg(test)]

use crate::{
    errors::ContractError,
    risk_gates::MAX_POSITIONS_PER_USER,
    TradeExecutorContract, TradeExecutorContractClient,
};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::Address as _,
    Address, Env,
};

/// Minimal UserPortfolio: open count + hooks expected by [`TradeExecutorContract::execute_copy_trade`].
#[contract]
pub struct MockUserPortfolio;

#[contracttype]
#[derive(Clone)]
enum MockKey {
    OpenCount(Address),
}

#[contractimpl]
impl MockUserPortfolio {
    pub fn get_open_position_count(env: Env, user: Address) -> u32 {
        env.storage()
            .instance()
            .get(&MockKey::OpenCount(user))
            .unwrap_or(0)
    }

    pub fn record_copy_position(env: Env, user: Address) {
        let key = MockKey::OpenCount(user.clone());
        let c: u32 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &(c + 1));
    }

    /// Decrement open count (simulates closing one copy position).
    pub fn close_one_copy_position(env: Env, user: Address) {
        let key = MockKey::OpenCount(user);
        let c: u32 = env.storage().instance().get(&key).unwrap_or(0);
        if c > 0 {
            env.storage().instance().set(&key, &(c - 1));
        }
    }
}

fn setup() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let portfolio_id = env.register(MockUserPortfolio, ());
    let exec_id = env.register(TradeExecutorContract, ());

    let exec = TradeExecutorContractClient::new(&env, &exec_id);
    exec.initialize(&admin);
    exec.set_user_portfolio(&portfolio_id);

    (env, exec_id, portfolio_id, user, admin)
}

#[test]
fn twenty_first_copy_trade_fails_until_one_closed() {
    let (env, exec_id, portfolio_id, user, _admin) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    for _ in 0..MAX_POSITIONS_PER_USER {
        exec.execute_copy_trade(&user);
    }

    let err = env.as_contract(&exec_id, || {
        crate::TradeExecutorContract::execute_copy_trade(env.clone(), user.clone())
    });
    assert_eq!(err, Err(ContractError::PositionLimitReached));

    MockUserPortfolioClient::new(&env, &portfolio_id).close_one_copy_position(&user);

    exec.execute_copy_trade(&user);

    let mock = MockUserPortfolioClient::new(&env, &portfolio_id);
    assert_eq!(mock.get_open_position_count(&user), MAX_POSITIONS_PER_USER);
}

#[test]
fn whitelisted_user_bypasses_position_limit() {
    let (env, exec_id, portfolio_id, user, _admin) = setup();
    let exec = TradeExecutorContractClient::new(&env, &exec_id);

    for _ in 0..MAX_POSITIONS_PER_USER {
        exec.execute_copy_trade(&user);
    }

    let err = env.as_contract(&exec_id, || {
        crate::TradeExecutorContract::execute_copy_trade(env.clone(), user.clone())
    });
    assert_eq!(err, Err(ContractError::PositionLimitReached));

    exec.set_position_limit_exempt(&user, &true);
    assert!(exec.is_position_limit_exempt(&user));

    exec.execute_copy_trade(&user);

    let mock = MockUserPortfolioClient::new(&env, &portfolio_id);
    assert_eq!(mock.get_open_position_count(&user), MAX_POSITIONS_PER_USER + 1);

    exec.set_position_limit_exempt(&user, &false);
    assert!(!exec.is_position_limit_exempt(&user));

    let err2 = env.as_contract(&exec_id, || {
        crate::TradeExecutorContract::execute_copy_trade(env.clone(), user.clone())
    });
    assert_eq!(err2, Err(ContractError::PositionLimitReached));
}
