use web3::types::{H160, U256};
use anyhow::{anyhow, Result};
use contracts::ethcontract::Bytes;
use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    pub(crate) token_in: H160,
    pub(crate) amount_in: U256,
    pub(crate) token_out: H160,
    pub(crate) amount_out: U256,
    pub(crate) valid_to: U256,
    pub(crate) maker: H160,
    pub(crate) uid: Bytes<Vec<u8>>,
}

// #[serde_as]
// #[derive(Debug, Derivative, Clone, Deserialize, Eq, PartialEq)]
// #[derivative(Default)]
// #[serde(rename_all = "camelCase")]
// pub struct Order {
//     /// The ID of the Ethereum chain where the `verifying_contract` is located.
//     pub chain_id: u64,
//     /// Timestamp in seconds of when the order expires. Expired orders cannot be
//     /// filled.
//     #[derivative(Default(value = "NaiveDateTime::MAX.timestamp() as u64"))]
//     #[serde_as(as = "DisplayFromStr")]
//     pub expiry: u64,
//     /// The address of the entity that will receive any fees stipulated by the
//     /// order. This is typically used to incentivize off-chain order relay.
//     pub fee_recipient: H160,
//     /// The address of the party that creates the order. The maker is also one
//     /// of the two parties that will be involved in the trade if the order
//     /// gets filled.
//     pub maker: H160,
//     /// The amount of `maker_token` being sold by the maker.
//     #[serde_as(as = "DisplayFromStr")]
//     pub maker_amount: u128,
//     /// The address of the ERC20 token the maker is selling to the taker.
//     pub maker_token: H160,
//     /// The staking pool to attribute the 0x protocol fee from this order. Set
//     /// to zero to attribute to the default pool, not owned by anyone.
//     pub pool: H256,
//     /// A value that can be used to guarantee order uniqueness. Typically it is
//     /// set to a random number.
//     #[serde(with = "u256_decimal")]
//     pub salt: U256,
//     /// It allows the maker to enforce that the order flow through some
//     /// additional logic before it can be filled (e.g., a KYC whitelist).
//     pub sender: H160,
//     /// The signature of the signed order.
//     pub signature: ZeroExSignature,
//     /// The address of the party that is allowed to fill the order. If set to a
//     /// specific party, the order cannot be filled by anyone else. If left
//     /// unspecified, anyone can fill the order.
//     pub taker: H160,
//     /// The amount of `taker_token` being sold by the taker.
//     #[serde_as(as = "DisplayFromStr")]
//     pub taker_amount: u128,
//     /// The address of the ERC20 token the taker is selling to the maker.
//     pub taker_token: H160,
//     /// Amount of takerToken paid by the taker to the feeRecipient.
//     #[serde_as(as = "DisplayFromStr")]
//     pub taker_token_fee_amount: u128,
//     /// Address of the contract where the transaction should be sent, usually
//     /// this is the 0x exchange proxy contract.
//     pub verifying_contract: H160,
// }
