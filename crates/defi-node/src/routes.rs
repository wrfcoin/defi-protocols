use std::{path::Path, sync::Arc};

use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use wrfcoin_amm::{pool::LiquidityPool, swap::SwapEngine};
use wrfcoin_defi_common::{Amount, AssetId, PoolId};

use crate::envelope::Envelope;

#[derive(Clone)]
pub struct AppState {
    pools: Arc<Vec<PoolView>>,
}

#[derive(Clone, Debug, Deserialize)]
struct PoolFixtureFile {
    pools: Vec<PoolFixture>,
}

#[derive(Clone, Debug, Deserialize)]
struct PoolFixture {
    id: u64,
    asset_a: String,
    asset_b: String,
    reserve_a: String,
    reserve_b: String,
    fee_rate_bps: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PoolView {
    pub id: u64,
    pub asset_a: String,
    pub asset_b: String,
    pub reserve_a: String,
    pub reserve_b: String,
    pub fee_rate_bps: u64,
}

impl PoolView {
    fn as_pool(&self) -> LiquidityPool {
        let mut pool = LiquidityPool::new(
            PoolId::new(self.id),
            AssetId::new(&self.asset_a),
            AssetId::new(&self.asset_b),
            self.fee_rate_bps,
        );
        pool.reserve_a = Amount::new(self.reserve_a.parse().expect("validated fixture reserve"));
        pool.reserve_b = Amount::new(self.reserve_b.parse().expect("validated fixture reserve"));
        pool
    }
}

#[derive(Debug, Serialize)]
struct HealthData {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct PoolsData {
    pools: Vec<PoolView>,
}

#[derive(Debug, Deserialize)]
pub struct QuoteRequest {
    amount_in: String,
    direction: Direction,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Direction {
    AToB,
    BToA,
}

#[derive(Debug, Serialize)]
struct QuoteData {
    pool_id: u64,
    amount_in: String,
    amount_out: String,
    direction: &'static str,
}

#[derive(Debug, Serialize)]
struct ErrorData {
    unavailable: bool,
    reason: String,
}

pub fn load_state(path: impl AsRef<Path>) -> Result<AppState, String> {
    let raw = std::fs::read_to_string(path.as_ref())
        .map_err(|error| format!("failed to read {}: {error}", path.as_ref().display()))?;
    let fixture: PoolFixtureFile =
        toml::from_str(&raw).map_err(|error| format!("invalid pool fixture: {error}"))?;
    if fixture.pools.is_empty() {
        return Err("pool fixture must contain at least one pool".into());
    }

    let pools = fixture
        .pools
        .into_iter()
        .map(|pool| {
            let reserve_a = pool
                .reserve_a
                .parse::<u128>()
                .map_err(|_| format!("pool {} reserve_a is not an integer", pool.id))?;
            let reserve_b = pool
                .reserve_b
                .parse::<u128>()
                .map_err(|_| format!("pool {} reserve_b is not an integer", pool.id))?;
            if reserve_a == 0 || reserve_b == 0 || pool.fee_rate_bps >= 10_000 {
                return Err(format!("pool {} has invalid reserves or fee", pool.id));
            }
            Ok(PoolView {
                id: pool.id,
                asset_a: pool.asset_a,
                asset_b: pool.asset_b,
                reserve_a: reserve_a.to_string(),
                reserve_b: reserve_b.to_string(),
                fee_rate_bps: pool.fee_rate_bps,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(AppState {
        pools: Arc::new(pools),
    })
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/defi/pools", get(list_pools))
        .route("/api/defi/pools/:id/quote", post(quote))
        .route("/api/defi/portfolio/:address", get(portfolio))
        .fallback(not_found)
        .with_state(state)
}

async fn health() -> Json<Envelope<HealthData>> {
    Json(Envelope::wrap(HealthData { status: "ok" }))
}

async fn list_pools(State(state): State<AppState>) -> Json<Envelope<PoolsData>> {
    Json(Envelope::wrap(PoolsData {
        pools: state.pools.as_ref().clone(),
    }))
}

async fn quote(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<u64>,
    Json(request): Json<QuoteRequest>,
) -> Response {
    let Some(view) = state.pools.iter().find(|pool| pool.id == id) else {
        return error(StatusCode::NOT_FOUND, format!("unknown pool {id}"));
    };
    let Ok(amount_in) = request.amount_in.parse::<u128>() else {
        return error(
            StatusCode::BAD_REQUEST,
            "amount_in must be an unsigned integer",
        );
    };
    if amount_in == 0 {
        return error(
            StatusCode::BAD_REQUEST,
            "amount_in must be greater than zero",
        );
    }

    let pool = view.as_pool();
    let (reserve_in, reserve_out, direction) = match request.direction {
        Direction::AToB => (pool.reserve_a, pool.reserve_b, "a_to_b"),
        Direction::BToA => (pool.reserve_b, pool.reserve_a, "b_to_a"),
    };
    match SwapEngine::new().get_amount_out(
        Amount::new(amount_in),
        reserve_in,
        reserve_out,
        pool.fee_rate_bps,
    ) {
        Ok(amount_out) => Json(Envelope::wrap(QuoteData {
            pool_id: id,
            amount_in: amount_in.to_string(),
            amount_out: amount_out.to_string(),
            direction,
        }))
        .into_response(),
        Err(error_value) => error(StatusCode::BAD_REQUEST, error_value.to_string()),
    }
}

async fn portfolio(AxumPath(_address): AxumPath<String>) -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(Envelope::wrap(ErrorData {
            unavailable: true,
            reason: "no ledger until the settlement ADR (Story D)".into(),
        })),
    )
        .into_response()
}

async fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "route not found")
}

fn error(status: StatusCode, reason: impl Into<String>) -> Response {
    (
        status,
        Json(Envelope::wrap(ErrorData {
            unavailable: true,
            reason: reason.into(),
        })),
    )
        .into_response()
}
