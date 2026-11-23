//! MeshCore backend for the Drawbridge host IPC.
//!
//! This is the first host vertical: it owns the sidecar HTTP client, the
//! durable claim/cursor store and the room bindings. Matrix delivery is the
//! next slice; until then `deliver` returns an explicit unsupported error
//! rather than pretending to send.

use crate::backend::{Backend, IpcResponse};
use anyhow::{Context, Result, bail};
use drawbridge_meshcore::{Conversation, MeshCoreClient};
use drawbridge_store::{Binding, Store};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub struct MeshCoreBackend {
    client: MeshCoreClient,
    store: Arc<Store>,
    tenant: String,
    cursor: Mutex<i64>,
}

impl MeshCoreBackend {
    pub fn new(client: MeshCoreClient, store: Arc<Store>, tenant: impl Into<String>) -> Self {
        let tenant = tenant.into();
        let cursor = store.cursor(&tenant, "meshcore").unwrap_or(0);
        Self {
            client,
            store,
            tenant,
            cursor: Mutex::new(cursor),
        }
    }

    fn tenant(&self) -> &str {
        &self.tenant
    }
}

#[async_trait::async_trait]
impl Backend for MeshCoreBackend {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "status" => Ok(serde_json::to_value(self.client.health().await?)?),
            "poll" => {
                let limit = params["limit"].as_u64().unwrap_or(100).min(500) as u32;
                let since = params["cursor"]
                    .as_i64()
                    .unwrap_or(*self.cursor.lock().unwrap());
                let messages = self.client.messages(since, limit).await?;
                let next = messages.iter().map(|m| m.id).max().unwrap_or(since);
                *self.cursor.lock().unwrap() = next;
                self.store.set_cursor(self.tenant(), "meshcore", next)?;
                Ok(json!({"messages": messages, "next": next}))
            }
            "send" => {
                let conversation = conversation(params.get("conversation"))?;
                let text = params["text"].as_str().context("send.text is required")?;
                let ack = self.client.send(&conversation, text).await?;
                Ok(serde_json::to_value(ack)?)
            }
            "advert" => {
                let flood = params["flood"].as_bool().unwrap_or(true);
                Ok(self.client.advert(flood).await?)
            }
            "settings" => Ok(self.client.settings(params).await?),
            "channel.create" => {
                let name = params["name"]
                    .as_str()
                    .context("channel.create.name is required")?;
                Ok(serde_json::to_value(
                    self.client.create_channel(name).await?,
                )?)
            }
            "channel.key" => {
                let index = params["index"]
                    .as_u64()
                    .context("channel.key.index is required")? as u8;
                Ok(serde_json::to_value(self.client.channel_key(index).await?)?)
            }
            "claim" => {
                let key = params["key"].as_str().context("claim.key is required")?;
                Ok(json!(self.store.claim_event(self.tenant(), key)?))
            }
            "bind" => {
                let binding: Binding = serde_json::from_value(params)?;
                self.store.upsert_binding(&binding)?;
                Ok(json!({}))
            }
            "bindings" => Ok(serde_json::to_value(self.store.bindings(self.tenant())?)?),
            "deliver" => bail!("Matrix delivery is not implemented in the MeshCore host yet"),
            "commit" => Ok(json!({})),
            "rollback" => Ok(json!({})),
            other => bail!("unknown host method {other}"),
        }
    }
}

fn conversation(value: Option<&Value>) -> Result<Conversation> {
    let value = value.context("send.conversation is required")?;
    if let Some(key) = value.as_str() {
        return Conversation::parse(key);
    }
    let kind = value["kind"]
        .as_str()
        .context("conversation.kind is required")?;
    let id = value["id"]
        .as_str()
        .context("conversation.id is required")?;
    Conversation::parse(&format!("{kind}:{id}"))
}

/// Convenience for adapters/tests.
pub async fn call(backend: &MeshCoreBackend, method: &str, params: Value) -> IpcResponse {
    crate::ipc::request(backend, method, params).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::IpcRequest;

    #[test]
    fn conversation_accepts_string_and_object() {
        assert_eq!(
            conversation(Some(&json!("channel:0"))).unwrap(),
            Conversation::Channel(0)
        );
        assert_eq!(
            conversation(Some(&json!({"kind":"contact","id":"dc6c06a8050b"}))).unwrap(),
            Conversation::Contact("dc6c06a8050b".into())
        );
        assert!(conversation(None).is_err());
    }

    #[test]
    fn ipc_request_round_trips() {
        let request: IpcRequest =
            serde_json::from_value(json!({"id":1,"method":"status","params":{}})).unwrap();
        assert_eq!(request.method, "status");
        let response = IpcResponse::ok(request.id, json!({"ok":true}));
        assert!(response.ok);
    }
}
