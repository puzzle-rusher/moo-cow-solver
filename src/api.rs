mod solve;
use std::convert::Infallible;
use warp::{hyper::StatusCode, Filter, Rejection, Reply};
use web3::transports::Http;
use web3::Web3;

use crate::slippage::SlippageCalculator;

pub fn handle_all_routes(
    slippage_calculator: SlippageCalculator,
    web3: Web3<Http>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    let solve = solve::get_solve(slippage_calculator, web3);
    let cors = warp::cors()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST", "DELETE", "OPTIONS", "PUT", "PATCH"])
        .allow_headers(vec!["Origin", "Content-Type", "X-Auth-Token", "X-AppId"]);
    solve.recover(handle_rejection).with(cors)
}
// We turn Rejection into Reply to workaround warp not setting CORS headers on rejections.
async fn handle_rejection(err: Rejection) -> Result<impl Reply, Infallible> {
    Ok(warp::reply::with_status(
        format!("{:?}", err),
        StatusCode::INTERNAL_SERVER_ERROR,
    ))
}
