use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One line on the IPC stream.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IpcRequest {
    /// Echoed back; useful for multiplexing over one stdio pipe.
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl IpcResponse {
    pub fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, error: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(error.into()),
        }
    }
}

/// The capability surface the adapter exposes to a Soil Spore.
#[async_trait::async_trait]
pub trait Backend: Send + Sync + 'static {
    async fn call(&self, method: &str, params: Value) -> Result<Value>;
}

pub async fn dispatch<B: Backend>(backend: &B, request: IpcRequest) -> IpcResponse {
    let id = request.id.clone();
    match backend.call(&request.method, request.params).await {
        Ok(result) => IpcResponse::ok(id, result),
        Err(error) => IpcResponse::error(id, format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Echo;

    #[async_trait::async_trait]
    impl Backend for Echo {
        async fn call(&self, method: &str, params: Value) -> Result<Value> {
            Ok(json!({"method": method, "params": params}))
        }
    }

    #[tokio::test]
    async fn dispatch_echoes_method_and_params() {
        let request = IpcRequest {
            id: Some(json!(7)),
            method: "status".into(),
            params: json!({"x": 1}),
        };
        let response = dispatch(&Echo, request).await;
        assert!(response.ok);
        assert_eq!(response.id, Some(json!(7)));
        assert_eq!(
            response.result,
            Some(json!({"method": "status", "params": {"x": 1}}))
        );
    }
}
