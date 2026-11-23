use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};
use drawbridge_rakazo::RakazoClient;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Seen {
    auth: Vec<String>,
    space: Vec<String>,
    path: Vec<String>,
}

async fn rpc(
    headers: HeaderMap,
    State(seen): State<Arc<Mutex<Seen>>>,
    axum::extract::Path(path): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut seen = seen.lock().unwrap();
    seen.auth.push(
        headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap()
            .into(),
    );
    seen.space.push(
        headers
            .get("x-rakazo-space-id")
            .unwrap()
            .to_str()
            .unwrap()
            .into(),
    );
    seen.path.push(path.clone());
    let reply = if path == "bots/list" {
        json!([{"id":"bot-1","name":"Muse","spaceId":"space"}])
    } else {
        json!({"messageId":"m-1","seq":7})
    };
    assert!(body["json"].is_object() || body["json"].is_null());
    Json(json!({"json": reply}))
}

#[tokio::test]
async fn client_sets_headers_and_unwraps_orpc_envelopes() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let app = Router::new()
        .route("/rpc/{*path}", post(rpc))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = RakazoClient::new(
        format!("http://{address}").parse().unwrap(),
        "token",
        "space",
    )
    .unwrap();
    let bots = client.bots().await.unwrap();
    assert_eq!(bots[0].id, "bot-1");
    let receipt = client
        .append_assistant("bot-1", "hello", "nonce-1", None)
        .await
        .unwrap();
    assert_eq!(receipt.message_id, "m-1");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.auth[0], "Bearer token");
    assert_eq!(seen.space[0], "space");
    assert_eq!(seen.path[0], "bots/list");
}
