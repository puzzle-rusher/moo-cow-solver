use {
    // contracts::IZeroEx,
    contracts::MooSettlementContract,
    crate::settlement_contract_data::Order,
    web3::types::Bytes,
};
use crate::interactions::{EncodedInteraction, Interaction};

#[derive(Clone, Debug)]
pub struct MooSettlementInteraction {
    pub order: Order,
    pub signature: Bytes,
    pub moo: MooSettlementContract,
}

impl Interaction for MooSettlementInteraction {
    fn encode(&self) -> Vec<EncodedInteraction> {
        let method = self.moo.swap(
            (
                self.order.token_in,
                self.order.amount_in,
                self.order.token_out,
                self.order.amount_out,
                self.order.valid_to,
                self.order.maker,
                self.order.uid.clone(),
            ),
            ethcontract::Bytes(self.signature.clone().0),
        );
        let calldata = method.tx.data.expect("no calldata").0;
        vec![(self.moo.address(), 0.into(), ethcontract::Bytes(calldata))]
    }
}
