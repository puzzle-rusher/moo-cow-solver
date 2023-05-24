pub mod api;
pub mod models;
pub mod slippage;
pub mod solve;
pub mod token_list;
pub mod tracing_helper;
pub mod utils;
mod interactions;
extern crate serde_derive;

#[macro_use]
extern crate lazy_static;

use slippage::SlippageCalculator;
use std::net::SocketAddr;
use tokio::{task, task::JoinHandle};
use web3::transports::Http;
use web3::Web3;

pub fn serve_task(address: SocketAddr, slippage_calculator: SlippageCalculator, web3: Web3<Http>) -> JoinHandle<()> {
    let filter = api::handle_all_routes(slippage_calculator, web3);
    tracing::info!(%address, "serving api");
    task::spawn(warp::serve(filter).bind(address))
}
