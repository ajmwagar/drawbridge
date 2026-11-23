//! Rakazo oRPC adapter.
//!
//! This is the transport-neutral request surface the host and the Rust bridge
//! share: bots, thread history, assistant replies and sends. Media, avatars and
//! Markdown stay in the Rakazo-specific adapter until they have a shared home.

use anyhow::{Context, Result, ensure};
use reqwest::{
    Client,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use url::Url;

#[derive(Clone)]
pub struct RakazoClient {
    client: Client,
    base: Url,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bot {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub parent_bot_id: Option<String>,
    #[serde(default)]
    pub spawn_key: Option<String>,
    #[serde(default)]
    pub archived_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub seq: i64,
    pub role: String,
    #[serde(default)]
    pub blocks: Value,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub client_nonce: Option<String>,
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub messages: Vec<Message>,
    #[serde(default)]
    pub older_cursor: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendReceipt {
    pub message_id: String,
    pub seq: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendReceipt {
    pub seq: i64,
    #[serde(default)]
    pub run_id: Option<String>,
}

impl RakazoClient {
    pub fn new(base: Url, token: &str, space: &str) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))?;
        auth.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth);
        headers.insert("x-rakazo-space-id", HeaderValue::from_str(space)?);
        Ok(Self {
            client: Client::builder()
                .default_headers(headers)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            base,
        })
    }

    /// oRPC JSON envelope: `{"json": input}` out, `body.json` back.
    pub async fn rpc<T: DeserializeOwned>(&self, path: &[&str], input: Value) -> Result<T> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid RPC base"))?
            .pop_if_empty()
            .extend(["rpc"])
            .extend(path);
        let response = self
            .client
            .post(url)
            .json(&json!({"json": input}))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Rakazo transport failed: {}", e.without_url()))?;
        ensure!(
            response.status().is_success(),
            "Rakazo RPC returned HTTP {}",
            response.status()
        );
        let body: Value = response.json().await.context("invalid Rakazo response")?;
        let value = body.get("json").context("missing oRPC json envelope")?;
        serde_json::from_value(value.clone()).context("Rakazo contract mismatch")
    }

    pub async fn bots(&self) -> Result<Vec<Bot>> {
        self.rpc(&["bots", "list"], Value::Null).await
    }

    pub async fn history_page(&self, bot: &str, before: i64) -> Result<Page> {
        self.rpc(
            &["threads", "messages"],
            json!({"botId": bot, "before": before}),
        )
        .await
    }

    /// Write an assistant message without starting a model run.
    pub async fn append_assistant(
        &self,
        bot: &str,
        text: &str,
        nonce: &str,
        reply_to: Option<&str>,
    ) -> Result<AppendReceipt> {
        let mut input = json!({"botId": bot, "text": text, "clientNonce": nonce});
        if let Some(reply) = reply_to {
            input["replyToMessageId"] = json!(reply);
        }
        self.rpc(&["threads", "appendAssistant"], input).await
    }

    pub async fn send(&self, bot: &str, text: &str, nonce: &str) -> Result<SendReceipt> {
        self.rpc(
            &["threads", "send"],
            json!({"botId": bot, "text": text, "artifactIds": [], "clientNonce": nonce}),
        )
        .await
    }

    pub async fn healthz(&self) -> Result<u16> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid RPC base"))?
            .pop_if_empty()
            .extend(["healthz"]);
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Rakazo health probe failed: {}", e.without_url()))?;
        Ok(response.status().as_u16())
    }
}
