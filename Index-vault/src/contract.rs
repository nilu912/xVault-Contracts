#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    to_json_binary, Addr, Binary, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult, Uint128, WasmMsg,
};
use cw2::set_contract_version;

use cw20::{Cw20ExecuteMsg, Denom, Expiration, MinterResponse};
use cw20_base::contract::query_balance;
use cw20_base::msg;
use serde::de;

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::{Config, Swapvar, BALANCE_OF, CONFIG, SWAPVAR, TOTAL_SUPPLY};

use wasmswap::msg::{
    ExecuteMsg as swapExecute, InstantiateMsg as swapInstantiateMSg, QueryMsg as swapQueryMsg,
    Token1ForToken2PriceResponse, Token2ForToken1PriceResponse, TokenSelect,
};

const CONTRACT_NAME: &str = "crates.io:cw-vault";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let owner = msg.owner_addr;
    let validate_owner = deps.api.addr_validate(&owner)?;
    let token = msg.token_addr;
    let validate_token = deps.api.addr_validate(&token)?;

    let lp_pool_1 = msg.lp_pool_1;
    let validate_lp_1 = deps.api.addr_validate(&lp_pool_1)?;

    let lp_pool_2 = msg.lp_pool_2;
    let validated_lp_2 = deps.api.addr_validate(&lp_pool_2)?;

    let rec_token_1 = msg.rec_token1;
    let validate_token_1 = deps.api.addr_validate(&rec_token_1)?;

    let rec_token_2 = msg.rec_token2;
    let validate_token_2 = deps.api.addr_validate(&rec_token_2)?;

    let config = Config {
        token: validate_token,
        owner: validate_owner,
    };

    let swapvar = Swapvar {
        lp_pool_1: validate_lp_1,
        rec_token_1: validate_token_1,
        lp_pool_2: validated_lp_2,
        rec_token_2: validate_token_2,
    };

    SWAPVAR.save(deps.storage, &swapvar)?;
    TOTAL_SUPPLY.save(deps.storage, &Uint128::zero())?;
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new().add_attribute("action", "Instantitate"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Deposit { amount } => execute_deposit(deps, env, info, amount),
        ExecuteMsg::Withdraw { share } => execute_withdraw(deps, env, info, share),
    }
}

fn execute_deposit(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount: Uint128,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let mut shares = Uint128::zero();
    let mut total_supply = TOTAL_SUPPLY.load(deps.storage)?;
    let mut balance = BALANCE_OF
        .load(deps.storage, info.sender.clone())
        .unwrap_or(Uint128::zero());

    let balance_contract =
        get_token_balance_of(&deps, env.contract.address.clone(), config.token.clone())?;

    if total_supply.is_zero() {
        shares = amount;
    } else {
        shares += amount
            .checked_mul(total_supply)
            .map_err(StdError::overflow)?
            .checked_div(balance_contract)
            .map_err(StdError::divide_by_zero)?;
    }

    total_supply += shares;
    TOTAL_SUPPLY.save(deps.storage, &total_supply)?;
    balance += shares;

    BALANCE_OF.save(deps.storage, info.sender.clone(), &balance)?;

    let transfer_cw20 = Cw20ExecuteMsg::TransferFrom {
        owner: info.sender.into(),
        recipient: env.contract.address.into(),
        amount: amount,
    };

    let msg = WasmMsg::Execute {
        contract_addr: config.token.clone().into(),
        msg: to_json_binary(&transfer_cw20)?,
        funds: vec![],
    };

    let c_msg: CosmosMsg = msg.into();
    let swapvar = SWAPVAR.load(deps.storage)?;

    let ratio = Uint128::new(2);

    let allow1 = get_cw20_increase_allowance_msg(&config.token, &swapvar.lp_pool_1, amount, None)?;

    let allow2 = get_cw20_increase_allowance_msg(&config.token, &swapvar.lp_pool_2, amount, None)?;

    let swap1 = swapExecute::Swap {
        input_token: TokenSelect::Token1,
        input_amount: amount
            .checked_div(ratio)
            .map_err(StdError::divide_by_zero)?,
        min_output: Uint128::zero(),
        expiration: None,
    };

    let swap_msg1 = WasmMsg::Execute {
        contract_addr: swapvar.lp_pool_1.into(),
        msg: to_json_binary(&swap1)?,
        funds: vec![],
    };

    let c_swap1: CosmosMsg = swap_msg1.into();

    let swap2 = swapExecute::Swap {
        input_token: TokenSelect::Token1,
        input_amount: amount
            .checked_div(ratio)
            .map_err(StdError::divide_by_zero)?,
        min_output: Uint128::zero(),
        expiration: None,
    };

    let swap_msg2 = WasmMsg::Execute {
        contract_addr: swapvar.lp_pool_2.into(),
        msg: to_json_binary(&swap2)?,
        funds: vec![],
    };

    let c_swap2: CosmosMsg = swap_msg2.into();

    Ok(Response::new()
        .add_message(allow1)
        .add_message(allow2)
        .add_message(c_msg)
        .add_message(c_swap1)
        .add_message(c_swap2))
}

fn execute_withdraw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    share: Uint128,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let token = config.token.clone();

    let swapvar = SWAPVAR.load(deps.storage)?;

    let mut total_supply = TOTAL_SUPPLY.load(deps.storage)?;

    let mut balance = BALANCE_OF
        .load(deps.storage, info.sender.clone())
        .unwrap_or(Uint128::zero());

    let token_1_bal = get_token_balance_of(
        &deps,
        env.contract.address.clone(),
        swapvar.rec_token_1.clone(),
    )?;

    let token_2_bal =
        get_token_balance_of(&deps, env.contract.address, swapvar.rec_token_2.clone())?;

    let am1: Uint128 = token_conversion(&deps, swapvar.lp_pool_1.clone(), token_1_bal)?;

    let am2: Uint128 = token_conversion(&deps, swapvar.lp_pool_2.clone(), token_2_bal)?;

    let token_bal: Uint128 = am1 + am2;

    let amount = share
        .checked_mul(token_bal)
        .map_err(StdError::overflow)?
        .checked_div(total_supply)
        .map_err(StdError::divide_by_zero)?;

    total_supply -= share;
    TOTAL_SUPPLY.save(deps.storage, &total_supply)?;
    balance -= share;
    BALANCE_OF.save(deps.storage, info.sender.clone(), &balance)?;

    let transfer_cw20 = Cw20ExecuteMsg::Transfer {
        recipient: info.sender.into(),
        amount: amount,
    };
    let msg = WasmMsg::Execute {
        contract_addr: config.token.into(),
        msg: to_json_binary(&transfer_cw20)?,
        funds: vec![],
    };

    let c_msg: CosmosMsg = msg.into();

    let allow1 = get_cw20_increase_allowance_msg(
        &swapvar.rec_token_1,
        &swapvar.lp_pool_1,
        token_1_bal,
        None,
    )?;

    let allow2 = get_cw20_increase_allowance_msg(
        &swapvar.rec_token_2,
        &swapvar.lp_pool_2,
        token_2_bal,
        None,
    )?;

    let swap1 = swapExecute::Swap {
        input_token: TokenSelect::Token1,
        input_amount: token_1_bal,
        min_output: Uint128::zero(),
        expiration: None,
    };

    let swap_msg1 = WasmMsg::Execute {
        contract_addr: swapvar.lp_pool_1.into(),
        msg: to_json_binary(&swap1)?,
        funds: vec![],
    };

    let c_swap1: CosmosMsg = swap_msg1.into();

    let swap2 = swapExecute::Swap {
        input_token: TokenSelect::Token2,
        input_amount: token_2_bal,
        min_output: Uint128::zero(),
        expiration: None,
    };

    let swap_msg2 = WasmMsg::Execute {
        contract_addr: swapvar.lp_pool_2.into(),
        msg: to_json_binary(&swap2)?,
        funds: vec![],
    };

    let c_swap2: CosmosMsg = swap_msg2.into();

    Ok(Response::new()
        .add_message(allow1)
        .add_message(allow2)
        .add_message(c_swap1)
        .add_message(c_swap2)
        .add_message(c_msg))
}

fn get_cw20_increase_allowance_msg(
    token_addr: &Addr,
    spender: &Addr,
    amount: Uint128,
    expires: Option<Expiration>,
) -> StdResult<CosmosMsg> {
    // create transfer cw20 msg
    let increase_allowance_msg = Cw20ExecuteMsg::IncreaseAllowance {
        spender: spender.to_string(),
        amount,
        expires,
    };
    let exec_allowance = WasmMsg::Execute {
        contract_addr: token_addr.into(),
        msg: to_json_binary(&increase_allowance_msg)?,
        funds: vec![],
    };
    Ok(exec_allowance.into())
}

pub fn get_token_balance_of(
    deps: &DepsMut,
    user_address: Addr,
    cw20_contract_addr: Addr,
) -> Result<Uint128, ContractError> {
    let resp: cw20::BalanceResponse = deps.querier.query_wasm_smart(
        cw20_contract_addr,
        &cw20_base::msg::QueryMsg::Balance {
            address: user_address.to_string(),
        },
    )?;
    Ok(resp.balance)
}

pub fn token_conversion(
    deps: &DepsMut,
    lp: Addr,
    amount: Uint128,
) -> Result<Uint128, ContractError> {
    let resp: Token2ForToken1PriceResponse = deps.querier.query_wasm_smart(
        lp,
        &swapQueryMsg::Token2ForToken1Price {
            token2_amount: amount,
        },
    )?;

    Ok(resp.token1_amount)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetTotalSupply {} => get_total_supply(deps),
        QueryMsg::GetBalanceOf { address } => get_balance_of(deps, address),
    }
}

fn get_total_supply(deps: Deps) -> StdResult<Binary> {
    let total = TOTAL_SUPPLY.load(deps.storage)?;

    return to_json_binary(&total);
}

fn get_balance_of(deps: Deps, address: Addr) -> StdResult<Binary> {
    let balance = BALANCE_OF.load(deps.storage, address)?;

    return to_json_binary(&balance);
}

#[cfg(test)]
mod tests {

    use crate::contract::{execute, instantiate};
    use crate::msg::{ExecuteMsg, InstantiateMsg};
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};

    pub const ADDR1: &str = "addr1";
    pub const ADDR2: &str = "addr2";

    #[test]

    fn test_instantiate() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let info = mock_info(ADDR1, &vec![]);

        let msg = InstantiateMsg {
            owner_addr: ADDR1.to_string(),
            token_addr: ADDR2.to_string(),
        };

        let res = instantiate(deps.as_mut(), env, info, msg).unwrap();

        println!("Deployed {:?}", res);
    }
}
