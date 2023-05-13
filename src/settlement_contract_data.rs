use primitive_types::{H160, U256};
use anyhow::{anyhow, Result};
use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Order {
    pub(crate) token_in: H160,
    pub(crate) amount_in: U256,
    pub(crate) token_out: H160,
    pub(crate) amount_out: U256,
    pub(crate) valid_to: U256,
    pub(crate) maker: H160,
    pub(crate) uid: Vec<u8>,
}

impl Order {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(Into::into)
    }
}
