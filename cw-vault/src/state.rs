use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Uint128};
use cw_storage_plus::{Item, Map};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub token: Addr,
    pub owner: Addr,
}

pub const CONFIG: Item<Config> = Item::new("Config");
pub const TOTAL_SUPPLY: Item<Uint128> = Item::new("total_supply");
pub const BALANCE_OF: Map<Addr, Uint128> = Map::new("balance_of");
