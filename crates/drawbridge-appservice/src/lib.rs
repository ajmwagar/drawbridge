//! Axum surface for a Matrix application service.
//!
//! The bridge supplies a [`TransactionHandler`]; this crate owns the boring,
//! security-sensitive part: bearer verification in constant time, the two
//! transaction paths (modern and legacy), bounded request bodies, and
//! health/readiness/metrics endpoints.

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, put},
};
use serde_json::{Value, json};
use std::sync::Arc;
use subtle::ConstantTimeEq;

/// A bridge's transaction sink.
#[async_trait::async_trait]
pub trait TransactionHandler: Send + Sync + 'static {
    /// Persist and acknowledge a transaction. Returning `Ok` is the durable
    /// acknowledgement; returning `Err` makes the homeserver retry.
    async fn ingest(&self, txn: &str, payload: &Value) -> Result<()>;
    fn ready(&self) -> bool {
        true
    }
    fn pending(&self) -> Option<u64> {
        None
    }
    /// Metric prefix, e.g. `meshcore_mautrix`.
    fn metric_prefix(&self) -> &'static str {
        "drawbridge"
    }
}

#[derive(Clone)]
pub struct AppServiceState {
    handler: Arc<dyn TransactionHandler>,
    hs_token: String,
}

impl AppServiceState {
    pub fn new(handler: Arc<dyn TransactionHandler>, hs_token: impl Into<String>) -> Self {
        Self {
            handler,
            hs_token: hs_token.into(),
        }
    }
}

pub fn router(state: AppServiceState) -> Router {
    Router::new()
        .route(
            "/healthz",
            get(|| async { Json(json!({"status":"alive"})) }),
        )
        .route("/readyz", get(ready))
        .route("/metrics", get(metrics))
        .route("/transactions/{txn}", put(transaction))
        .route("/_matrix/app/v1/transactions/{txn}", put(transaction))
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
        .with_state(state)
}

async fn ready(State(state): State<AppServiceState>) -> impl IntoResponse {
    let ready = state.handler.ready();
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(json!({"ready": ready})),
    )
}

async fn metrics(State(state): State<AppServiceState>) -> impl IntoResponse {
    let prefix = state.handler.metric_prefix();
    let pending = state.handler.pending();
    let mut body = format!(
        "# TYPE {prefix}_ready gauge\n{prefix}_ready {}\n",
        u8::from(state.handler.ready())
    );
    if let Some(pending) = pending {
        body.push_str(&format!(
            "# TYPE {prefix}_pending gauge\n{prefix}_pending {pending}\n"
        ));
    }
    (StatusCode::OK, body)
}

async fn transaction(
    State(state): State<AppServiceState>,
    Path(txn): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or("");
    if !bool::from(token.as_bytes().ct_eq(state.hs_token.as_bytes())) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"errcode":"M_UNAUTHORIZED"})),
        );
    }
    if txn.is_empty() || txn.len() > 1024 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"errcode":"M_INVALID_PARAM","error":"invalid transaction id"})),
        );
    }
    match state.handler.ingest(&txn, &body).await {
        Ok(()) => (StatusCode::OK, Json(json!({}))),
        Err(error) => {
            tracing::error!(%txn, %error, "transaction not acknowledged");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"errcode":"M_UNKNOWN","error":"transaction not committed"})),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    struct Sink(AtomicUsize);

    #[async_trait::async_trait]
    impl TransactionHandler for Sink {
        async fn ingest(&self, _txn: &str, _payload: &Value) -> Result<()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn pending(&self) -> Option<u64> {
            Some(self.0.load(Ordering::Relaxed) as u64)
        }
    }

    fn app(token: &str) -> (Router, Arc<Sink>) {
        let sink = Arc::new(Sink(AtomicUsize::new(0)));
        let state = AppServiceState::new(sink.clone(), token);
        (router(state), sink)
    }

    #[tokio::test]
    async fn rejects_missing_and_wrong_bearer() {
        let (app, _) = app("hs-secret");
        for header in [None, Some("Bearer wrong")] {
            let mut request = Request::builder()
                .method("PUT")
                .uri("/_matrix/app/v1/transactions/t1");
            if let Some(header) = header {
                request = request.header("authorization", header);
            }
            let response = app
                .clone()
                .oneshot(
                    request
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"events":[]}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn accepts_both_transaction_paths() {
        let (app, sink) = app("hs-secret");
        for path in ["/transactions/t2", "/_matrix/app/v1/transactions/t3"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("PUT")
                        .uri(path)
                        .header("authorization", "Bearer hs-secret")
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"events":[]}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        assert_eq!(sink.0.load(Ordering::Relaxed), 2);
    }
}
