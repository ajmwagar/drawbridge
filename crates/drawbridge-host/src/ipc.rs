use crate::backend::{Backend, IpcRequest, IpcResponse, dispatch};
use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Mutex,
};

/// Serve line-delimited JSON requests until the input closes.
///
/// The request stream is the `loam:plugin/ipc` side of the adapter. Each line
/// is one JSON object; each response is one JSON object on one line. A
/// malformed line gets an error response and does not kill the session.
pub async fn serve<B, R, W>(backend: Arc<B>, reader: R, writer: W) -> Result<()>
where
    B: Backend,
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    let writer = Arc::new(Mutex::new(writer));
    while let Some(line) = lines.next_line().await.context("read IPC line")? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<IpcRequest>(&line) {
            Ok(request) => dispatch(backend.as_ref(), request).await,
            Err(error) => IpcResponse::error(None, format!("invalid IPC request: {error}")),
        };
        let mut writer = writer.lock().await;
        writer
            .write_all(serde_json::to_string(&response)?.as_bytes())
            .await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
    }
    Ok(())
}

/// Convenience for adapters that only need one request/response.
pub async fn request<B: Backend>(backend: &B, method: &str, params: Value) -> IpcResponse {
    dispatch(
        backend,
        IpcRequest {
            id: None,
            method: method.to_string(),
            params,
        },
    )
    .await
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
    async fn serves_one_response_per_line_and_survives_bad_json() {
        use tokio::io::{AsyncWriteExt, BufReader};

        let (mut client, server) = tokio::io::duplex(4096);
        let (read, write) = tokio::io::split(server);
        let backend = Arc::new(Echo);
        let task = tokio::spawn(async move { serve(backend, read, write).await });

        client
            .write_all(b"{\"method\":\"status\",\"params\":{}}\n")
            .await
            .unwrap();
        client.write_all(b"not json\n").await.unwrap();
        client
            .write_all(b"{\"method\":\"send\",\"params\":{\"text\":\"hi\"}}\n")
            .await
            .unwrap();
        client.shutdown().await.unwrap();

        let mut lines = BufReader::new(client).lines();
        let first: IpcResponse =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert!(first.ok);
        let second: IpcResponse =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert!(!second.ok);
        let third: IpcResponse =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert!(third.ok);
        task.await.unwrap().unwrap();
    }
}
