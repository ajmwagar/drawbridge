//! Rakazo backend for the Drawbridge host IPC.
//!
//! Polling reads the latest history page per configured bot and advances a
//! durable per-bot cursor. Sends use the assistant-append path so a Matrix
//! message never starts a model run by accident. Matrix delivery is the next
//! slice, as with MeshCore.

use crate::backend::Backend;
use anyhow::{Context, Result, bail};
use drawbridge_rakazo::RakazoClient;
use drawbridge_store::{Binding, Store};
use serde_json::{Value, json};
use std::sync::Arc;

pub struct RakazoBackend {
    client: RakazoClient,
    store: Arc<Store>,
    tenant: String,
    bots: Vec<String>,
}

impl RakazoBackend {
    pub fn new(
        client: RakazoClient,
        store: Arc<Store>,
        tenant: impl Into<String>,
        bots: Vec<String>,
    ) -> Self {
        Self {
            client,
            store,
            tenant: tenant.into(),
            bots,
        }
    }
}

#[async_trait::async_trait]
impl Backend for RakazoBackend {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "status" => {
                let code = self.client.healthz().await?;
                let bots = self.client.bots().await?;
                Ok(json!({"http": code, "bots": bots.len(), "configured": self.bots}))
            }
            "bots" => Ok(serde_json::to_value(self.client.bots().await?)?),
            "poll" => {
                let mut events = Vec::new();
                for bot in &self.bots {
                    let cursor = self.store.cursor(&self.tenant, &format!("rakazo:{bot}"))?;
                    let page = self.client.history_page(bot, i64::MAX).await?;
                    let mut next = cursor;
                    for message in page.messages.into_iter().filter(|m| m.seq > cursor) {
                        next = next.max(message.seq);
                        events.push(json!({"bot": bot, "message": message}));
                    }
                    if next > cursor {
                        self.store
                            .set_cursor(&self.tenant, &format!("rakazo:{bot}"), next)?;
                    }
                }
                events.sort_by_key(|event| event["message"]["seq"].as_i64().unwrap_or(0));
                Ok(json!({"events": events}))
            }
            "send" => {
                let bot = params["bot"].as_str().context("send.bot is required")?;
                let text = params["text"].as_str().context("send.text is required")?;
                let nonce = params["nonce"].as_str().context("send.nonce is required")?;
                let reply = params["reply"].as_str();
                let receipt = self
                    .client
                    .append_assistant(bot, text, nonce, reply)
                    .await?;
                Ok(serde_json::to_value(receipt)?)
            }
            "claim" => {
                let key = params["key"].as_str().context("claim.key is required")?;
                Ok(json!(self.store.claim_event(&self.tenant, key)?))
            }
            "bind" => {
                let binding: Binding = serde_json::from_value(params)?;
                self.store.upsert_binding(&binding)?;
                Ok(json!({}))
            }
            "bindings" => Ok(serde_json::to_value(self.store.bindings(&self.tenant)?)?),
            "deliver" => bail!("Matrix delivery is not implemented in the Rakazo host yet"),
            "commit" => Ok(json!({})),
            "rollback" => Ok(json!({})),
            other => bail!("unknown host method {other}"),
        }
    }
}
