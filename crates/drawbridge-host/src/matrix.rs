//! Matrix delivery for a Drawbridge host.
//!
//! The layer owns the appservice HTTP surface and persists inbound transactions
//! before acknowledging them. `poll` appends those transactions to the network
//! backend's events; the Spore calls `deliver`, `ensure-room` and `commit`.

use crate::backend::{Backend, IpcResponse};
use anyhow::{Context, Result};
use async_trait::async_trait;
use drawbridge_appservice::TransactionHandler;
use drawbridge_core::{RoomSpec, secret};
use drawbridge_matrix::{MatrixAppService, MatrixUser, RegistrationBundle};
use drawbridge_store::{Binding, Store};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use url::Url;

#[derive(Clone, Debug, Deserialize)]
pub struct MatrixConfig {
    pub listen: SocketAddr,
    pub owner: String,
    pub homeserver: Url,
    pub registration: PathBuf,
    #[serde(default)]
    pub owner_token_env: Option<String>,
}

pub struct MatrixLayer<B> {
    backend: B,
    matrix: MatrixAppService,
    owner: Option<MatrixUser>,
    store: Arc<Store>,
    tenant: String,
}

impl<B: Backend> MatrixLayer<B> {
    pub fn new(
        config: &MatrixConfig,
        backend: B,
        store: Arc<Store>,
        tenant: impl Into<String>,
    ) -> Result<(Arc<Self>, String)> {
        let bundle = RegistrationBundle::load(&config.registration)?;
        let matrix = MatrixAppService::new(&config.homeserver, &bundle)?;
        let owner = match config.owner_token_env.as_deref() {
            Some(env) => Some(MatrixUser::new(&config.homeserver, secret(env)?)?),
            None => None,
        };
        let hs_token = bundle.registration.hs_token.clone();
        Ok((
            Arc::new(Self {
                backend,
                matrix,
                owner,
                store,
                tenant: tenant.into(),
            }),
            hs_token,
        ))
    }

    async fn ensure_room(&self, params: Value) -> Result<Value> {
        let conversation = params
            .get("conversation")
            .context("ensure-room.conversation is required")?;
        let kind = conversation["kind"]
            .as_str()
            .context("conversation.kind is required")?;
        let id = conversation["id"]
            .as_str()
            .context("conversation.id is required")?;
        let display = params["display"]
            .as_str()
            .context("ensure-room.display is required")?;
        let protocol = params["protocol"].as_str().unwrap_or("drawbridge");
        let protocol_name = params["protocol-name"].as_str().unwrap_or(protocol);
        let key = format!("{kind}:{id}");
        let ghost = self.matrix.ghost_for(&[&self.tenant, &key])?;
        self.matrix.register_user(&ghost, display).await?;
        let spec = RoomSpec {
            name: format!("{display} · {protocol_name}"),
            topic: "Bridge room. This room is not end-to-end encrypted.".into(),
            bridge_key: format!("{protocol}://{}/{key}", self.tenant),
            protocol_id: protocol.to_string(),
            protocol_display_name: protocol_name.to_string(),
            channel_id: id.to_string(),
            channel_display_name: display.to_string(),
        };
        let room = self
            .matrix
            .ensure_room(&spec, &ghost, params["owner"].as_str().unwrap_or(""))
            .await?;
        if let Some(owner) = &self.owner
            && let Err(error) = owner.join(&room).await
        {
            tracing::warn!(%error, %room, "owner auto-join failed");
        }
        let binding = Binding {
            tenant: self.tenant.clone(),
            key,
            room,
            ghost,
            display: display.to_string(),
        };
        self.store.upsert_binding(&binding)?;
        Ok(serde_json::to_value(&binding)?)
    }
}

#[async_trait]
impl<B: Backend> Backend for MatrixLayer<B> {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "poll" => {
                let limit = params["limit"].as_u64().unwrap_or(100).min(500) as u32;
                let mut result = self.backend.call(method, params).await?;
                let pending = self.store.pending_inbound(&self.tenant, limit)?;
                if !pending.is_empty() {
                    let events = result
                        .get_mut("events")
                        .and_then(Value::as_array_mut)
                        .context("backend poll result missing events array")?;
                    for row in pending {
                        let payload: Value = serde_json::from_slice(&row.payload)?;
                        events.push(json!({
                            "matrix": {
                                "id": row.event_id,
                                "events": payload["events"].clone()
                            }
                        }));
                    }
                }
                Ok(result)
            }
            "deliver" => {
                let room = params["room"]
                    .as_str()
                    .context("deliver.room is required")?;
                let transaction = params["transaction"]
                    .as_str()
                    .context("deliver.transaction is required")?;
                let text = params["text"]
                    .as_str()
                    .context("deliver.text is required")?;
                let event_id = self
                    .matrix
                    .send_text(room, &self.matrix.sender, transaction, text)
                    .await?;
                Ok(json!({"event": event_id}))
            }
            "ensure-room" => self.ensure_room(params).await,
            "bind" => {
                let binding: Binding = serde_json::from_value(params)?;
                self.store.upsert_binding(&binding)?;
                Ok(json!({}))
            }
            "bindings" => Ok(serde_json::to_value(self.store.bindings(&self.tenant)?)?),
            "claim" => {
                let key = params["key"].as_str().context("claim.key is required")?;
                Ok(json!(self.store.claim_event(&self.tenant, key)?))
            }
            "commit" => {
                let transaction = params["transaction"]
                    .as_str()
                    .context("commit.transaction is required")?;
                self.store.complete_inbound(&self.tenant, transaction)?;
                Ok(json!({}))
            }
            "rollback" => {
                let transaction = params["transaction"]
                    .as_str()
                    .context("rollback.transaction is required")?;
                self.store.retry_inbound(&self.tenant, transaction)?;
                Ok(json!({}))
            }
            "status" => {
                let mut result = self.backend.call(method, params).await?;
                result["matrix"] = json!(true);
                Ok(result)
            }
            _ => self.backend.call(method, params).await,
        }
    }
}

#[async_trait]
impl<B: Backend> TransactionHandler for MatrixLayer<B> {
    async fn ingest(&self, txn: &str, payload: &Value) -> Result<()> {
        let bytes = serde_json::to_vec(payload)?;
        self.store
            .queue_inbound(&self.tenant, txn, "matrix", &bytes)?;
        Ok(())
    }

    fn ready(&self) -> bool {
        true
    }

    fn pending(&self) -> Option<u64> {
        self.store.pending_count().ok()
    }

    fn metric_prefix(&self) -> &'static str {
        "drawbridge_host"
    }
}

/// Helper for binaries/tests that only need one IPC response.
pub async fn call<B: Backend>(layer: &MatrixLayer<B>, method: &str, params: Value) -> IpcResponse {
    crate::ipc::request(layer, method, params).await
}

impl<B: Backend> MatrixLayer<B> {
    pub fn matrix(&self) -> &MatrixAppService {
        &self.matrix
    }
}
