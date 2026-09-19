use std::path::PathBuf;

use defi_node::{load_state, router};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tokio::task::JoinHandle;
use wrfcoin_amm::swap::SwapEngine;
use wrfcoin_defi_common::Amount;

async fn server() -> (String, JoinHandle<()>) {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/pools.toml");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = router(load_state(fixture).unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), task)
}

fn assert_simulator(body: &Value) {
    assert_eq!(body["simulated"], true);
    assert_eq!(body["custodial"], false);
    assert_eq!(body["settlement"], "none");
}

#[tokio::test]
async fn health_ok() {
    let (base, task) = server().await;
    let response = reqwest::get(format!("{base}/health")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_simulator(&body);
    assert_eq!(body["status"], "ok");
    task.abort();
}

#[tokio::test]
async fn pools_lists_fixture_and_is_simulated() {
    let (base, task) = server().await;
    let body: Value = reqwest::get(format!("{base}/api/defi/pools"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_simulator(&body);
    assert_eq!(body["pools"].as_array().unwrap().len(), 2);
    task.abort();
}

#[tokio::test]
async fn quote_matches_swap_engine() {
    let (base, task) = server().await;
    let body: Value = Client::new()
        .post(format!("{base}/api/defi/pools/1/quote"))
        .json(&json!({"amount_in":"100000000", "direction":"a_to_b"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let expected = SwapEngine::new()
        .get_amount_out(
            Amount::new(100_000_000),
            Amount::new(100_000_000_000),
            Amount::new(250_000_000_000),
            30,
        )
        .unwrap();
    assert_simulator(&body);
    assert_eq!(body["amount_out"], expected.to_string());
    task.abort();
}

#[tokio::test]
async fn quote_zero_amount_is_400() {
    let (base, task) = server().await;
    let response = Client::new()
        .post(format!("{base}/api/defi/pools/1/quote"))
        .json(&json!({"amount_in":"0", "direction":"a_to_b"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_simulator(&response.json().await.unwrap());
    task.abort();
}

#[tokio::test]
async fn quote_unknown_pool_is_404() {
    let (base, task) = server().await;
    let response = Client::new()
        .post(format!("{base}/api/defi/pools/999/quote"))
        .json(&json!({"amount_in":"1", "direction":"a_to_b"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_simulator(&response.json().await.unwrap());
    task.abort();
}

#[tokio::test]
async fn portfolio_is_501_unavailable() {
    let (base, task) = server().await;
    let response = reqwest::get(format!("{base}/api/defi/portfolio/0xabc"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    let body: Value = response.json().await.unwrap();
    assert_simulator(&body);
    assert_eq!(body["unavailable"], true);
    task.abort();
}

#[tokio::test]
async fn every_non_error_body_has_simulated_true() {
    let (base, task) = server().await;
    let client = Client::new();
    let mut bodies = Vec::new();
    bodies.push(
        client
            .get(format!("{base}/health"))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap(),
    );
    bodies.push(
        client
            .get(format!("{base}/api/defi/pools"))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap(),
    );
    bodies.push(
        client
            .post(format!("{base}/api/defi/pools/1/quote"))
            .json(&json!({"amount_in":"1", "direction":"b_to_a"}))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap(),
    );
    for body in bodies {
        assert_simulator(&body);
    }
    task.abort();
}

#[tokio::test]
async fn no_route_accepts_state_change() {
    let (base, task) = server().await;
    let client = Client::new();
    for path in [
        "/api/defi/pools",
        "/api/defi/pools/1/quote",
        "/api/defi/portfolio/0xabc",
    ] {
        for method in [
            reqwest::Method::PUT,
            reqwest::Method::PATCH,
            reqwest::Method::DELETE,
        ] {
            let status = client
                .request(method, format!("{base}{path}"))
                .send()
                .await
                .unwrap()
                .status();
            assert!(status == StatusCode::METHOD_NOT_ALLOWED || status == StatusCode::NOT_FOUND);
        }
    }
    task.abort();
}
