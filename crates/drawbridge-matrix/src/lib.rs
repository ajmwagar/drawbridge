//! Matrix application-service client primitives.
//!
//! Adapted from `FuturePresentLabs/rakazo-beeper`'s `src/matrix.rs`, with the
//! Rakazo/Beeper-specific room, avatar and thread policy removed. The remaining
//! surface is the generic part every Drawbridge bridge needs: registration
//! bundles, ghost namespacing, room creation, idempotent sends, media upload and
//! download, and transaction parsing.
//!
//! References:
//! - <https://spec.matrix.org/v1.16/application-service-api/>
//! - <https://spec.matrix.org/v1.16/client-server-api/>

use anyhow::{Context, Result, ensure};
use drawbridge_core::{InboundEvent, RoomSpec, stable_id};
use regex::Regex;
use reqwest::{
    Client, Method, StatusCode,
    header::{AUTHORIZATION, HeaderValue},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use url::Url;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Registration {
    pub id: String,
    pub as_token: String,
    pub hs_token: String,
    pub sender_localpart: String,
    pub namespaces: Namespaces,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Namespaces {
    pub users: Vec<Namespace>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Namespace {
    pub regex: String,
    pub exclusive: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RegistrationBundle {
    pub registration: Registration,
    pub homeserver_url: Url,
    pub homeserver_domain: String,
    pub your_user_id: String,
}

impl RegistrationBundle {
    pub fn load(path: &Path) -> Result<Self> {
        let bundle: Self = serde_json::from_str(
            &std::fs::read_to_string(path)
                .with_context(|| format!("read registration bundle {}", path.display()))?,
        )
        .context("registration must be the full `bbctl register --json` response")?;
        ensure!(
            !bundle.homeserver_domain.is_empty(),
            "registration domain missing"
        );
        ensure!(
            bundle.your_user_id.starts_with('@') && bundle.your_user_id.contains(':'),
            "registration owner is not a Matrix user id"
        );
        ensure!(
            bundle.registration.as_token.len() >= 20 && bundle.registration.hs_token.len() >= 20,
            "registration tokens too short"
        );
        Ok(bundle)
    }
}

/// One event from an appservice transaction, normalized but not filtered.
#[derive(Clone, Debug)]
pub struct TransactionEvent {
    pub event_id: String,
    pub room_id: String,
    pub sender: String,
    pub kind: String,
    pub content: Value,
    pub origin_server_ts: Option<u64>,
}

impl TransactionEvent {
    pub fn encrypted(&self) -> bool {
        self.kind == "m.room.encrypted"
    }

    /// The plain-text body of a normal `m.room.message`, if this is one.
    pub fn text(&self) -> Option<&str> {
        if self.kind != "m.room.message" {
            return None;
        }
        self.content.get("body").and_then(Value::as_str)
    }

    pub fn as_inbound(&self, conversation: &str) -> Option<InboundEvent> {
        Some(InboundEvent {
            remote_id: self.event_id.clone(),
            conversation: conversation.to_string(),
            sender: self.sender.clone(),
            sender_name: None,
            body: self.text()?.to_string(),
            sent_at: self.origin_server_ts.map(|ms| ms / 1000),
        })
    }
}

/// Parse an appservice transaction body. Malformed transaction shapes are
/// rejected; individual malformed events are skipped because one bad event must
/// not block an entire transaction.
pub fn parse_transaction(payload: &Value) -> Result<Vec<TransactionEvent>> {
    let events = payload
        .get("events")
        .and_then(Value::as_array)
        .context("transaction missing events array")?;
    ensure!(events.len() <= 10_000, "transaction too large");
    let mut parsed = Vec::with_capacity(events.len());
    for event in events {
        let Some(event_id) = event.get("event_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(room_id) = event.get("room_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(sender) = event.get("sender").and_then(Value::as_str) else {
            continue;
        };
        let Some(kind) = event.get("type").and_then(Value::as_str) else {
            continue;
        };
        parsed.push(TransactionEvent {
            event_id: event_id.to_string(),
            room_id: room_id.to_string(),
            sender: sender.to_string(),
            kind: kind.to_string(),
            content: event.get("content").cloned().unwrap_or(Value::Null),
            origin_server_ts: event.get("origin_server_ts").and_then(Value::as_u64),
        });
    }
    Ok(parsed)
}

#[derive(Clone)]
pub struct MatrixAppService {
    client: Client,
    base: Url,
    token: HeaderValue,
    pub domain: String,
    pub sender: String,
    pub prefix: String,
    namespaces: Vec<Regex>,
}

impl MatrixAppService {
    pub fn new(homeserver: &Url, bundle: &RegistrationBundle) -> Result<Self> {
        ensure!(
            bundle.homeserver_url == *homeserver,
            "homeserver differs from registration bundle"
        );
        let registration = &bundle.registration;
        let mut token = HeaderValue::from_str(&format!("Bearer {}", registration.as_token))?;
        token.set_sensitive(true);
        let sender = format!(
            "@{}:{}",
            registration.sender_localpart, bundle.homeserver_domain
        );
        let prefix = registration
            .sender_localpart
            .strip_suffix("bot")
            .unwrap_or(&registration.sender_localpart)
            .to_string();
        ensure!(
            !prefix.is_empty(),
            "sender_localpart yields an empty prefix"
        );
        let namespaces = registration
            .namespaces
            .users
            .iter()
            .filter(|ns| ns.exclusive)
            .map(|ns| Regex::new(&format!("^(?:{})$", ns.regex)))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ensure!(!namespaces.is_empty(), "exclusive user namespace required");
        Ok(Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            base: homeserver.clone(),
            token,
            domain: bundle.homeserver_domain.clone(),
            sender,
            prefix,
            namespaces,
        })
    }

    /// Stable ghost for a bridge-local identity. The namespace is checked
    /// before the name ever reaches Matrix.
    pub fn ghost(&self, stable: &str) -> Result<String> {
        let user = format!("@{}_{}:{}", self.prefix, stable, self.domain);
        ensure!(
            self.namespaces.iter().any(|ns| ns.is_match(&user)),
            "ghost is outside the registered namespace: {user}"
        );
        Ok(user)
    }

    pub fn ghost_for(&self, parts: &[&str]) -> Result<String> {
        self.ghost(&stable_id(parts))
    }

    fn url(&self, path: &[&str]) -> Result<Url> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid Matrix base URL"))?
            .pop_if_empty()
            .extend(["_matrix", "client", "v3"])
            .extend(path);
        Ok(url)
    }

    async fn request(
        &self,
        method: Method,
        path: &[&str],
        user: Option<&str>,
        body: Option<Value>,
    ) -> Result<(StatusCode, Value)> {
        let mut url = self.url(path)?;
        if let Some(user) = user {
            url.query_pairs_mut().append_pair("user_id", user);
        }
        let mut request = self
            .client
            .request(method, url)
            .header(AUTHORIZATION, self.token.clone());
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Matrix transport failed: {}", e.without_url()))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        Ok((status, body))
    }

    async fn ok(
        &self,
        method: Method,
        path: &[&str],
        user: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let (status, body) = self.request(method, path, Some(user), body).await?;
        ensure!(
            status.is_success(),
            "Matrix request returned HTTP {status}: {body}"
        );
        Ok(body)
    }

    /// Register the appservice's own bot user.
    pub async fn register_bot(&self, displayname: &str) -> Result<()> {
        self.register_user(&self.sender, displayname).await
    }

    /// Register an application-service user. `M_USER_IN_USE` is success because
    /// registration is idempotent and the ghost may already exist.
    pub async fn register_user(&self, user: &str, displayname: &str) -> Result<()> {
        let localpart = user
            .strip_prefix('@')
            .and_then(|s| s.split_once(':'))
            .context("invalid ghost")?
            .0;
        let (status, body) = self
            .request(
                Method::POST,
                &["register"],
                None,
                Some(json!({
                    "type": "m.login.application_service",
                    "username": localpart,
                    "inhibit_login": true
                })),
            )
            .await?;
        ensure!(
            status.is_success()
                || (status == StatusCode::BAD_REQUEST && body["errcode"] == "M_USER_IN_USE"),
            "Matrix ghost registration returned HTTP {status}: {body}"
        );
        self.set_displayname(user, displayname).await
    }

    pub async fn set_displayname(&self, user: &str, name: &str) -> Result<()> {
        self.ok(
            Method::PUT,
            &["profile", user, "displayname"],
            user,
            Some(json!({"displayname": name})),
        )
        .await?;
        Ok(())
    }

    pub async fn joined_rooms(&self, user: &str) -> Result<Vec<String>> {
        let joined = self.ok(Method::GET, &["joined_rooms"], user, None).await?;
        Ok(joined["joined_rooms"]
            .as_array()
            .context("Matrix joined_rooms missing array")?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect())
    }

    /// Find an existing room by its `m.bridge` state key. This recovers a room
    /// if the local database loses a binding but Matrix still has the room.
    pub async fn find_room(&self, ghost: &str, bridge_key: &str) -> Result<Option<String>> {
        let mut matches = Vec::new();
        for room in self.joined_rooms(ghost).await? {
            let state = self
                .ok(Method::GET, &["rooms", &room, "state"], ghost, None)
                .await?;
            if state.as_array().is_some_and(|events| {
                events.iter().any(|event| {
                    event["type"] == "m.bridge"
                        && event["state_key"] == bridge_key
                        && event["content"]["creator"] == ghost
                })
            }) {
                matches.push(room);
            }
        }
        ensure!(
            matches.len() <= 1,
            "multiple rooms match {bridge_key}; delivery paused"
        );
        Ok(matches.pop())
    }

    /// Create an unencrypted private room owned by `ghost` and invite `owner`.
    pub async fn create_room(&self, spec: &RoomSpec, ghost: &str, owner: &str) -> Result<String> {
        ensure!(
            spec.bridge_key
                .starts_with(&format!("{}://", spec.protocol_id)),
            "bridge key must start with the protocol id"
        );
        let bridge = json!({
            "bridgebot": self.sender,
            "creator": ghost,
            "protocol": {"id": spec.protocol_id, "displayname": spec.protocol_display_name},
            "channel": {"id": spec.channel_id, "displayname": spec.channel_display_name}
        });
        let body = self
            .ok(
                Method::POST,
                &["createRoom"],
                ghost,
                Some(json!({
                    "name": spec.name,
                    "topic": spec.topic,
                    "visibility": "private",
                    "preset": "private_chat",
                    "invite": [owner],
                    "creation_content": {"m.federate": false},
                    "power_level_content_override": {
                        "users": {ghost: 100, owner: 0},
                        "users_default": 0,
                        "events_default": 0,
                        "state_default": 100,
                        "invite": 100,
                        "kick": 100,
                        "ban": 100,
                        "redact": 100,
                        "events": {
                            "m.room.message": 0,
                            "m.reaction": 0,
                            "m.sticker": 0,
                            "m.room.encryption": 100
                        }
                    },
                    "initial_state": [
                        {
                            "type": "m.room.history_visibility",
                            "state_key": "",
                            "content": {"history_visibility": "joined"}
                        },
                        {"type": "m.bridge", "state_key": spec.bridge_key, "content": bridge},
                        {"type": "uk.half-shot.bridge", "state_key": spec.bridge_key, "content": bridge}
                    ]
                })),
            )
            .await?;
        body["room_id"]
            .as_str()
            .context("Matrix createRoom missing room_id")
            .map(str::to_string)
    }

    /// Ensure a room exists and is bound, reusing Matrix state when the local
    /// database lost the binding.
    pub async fn ensure_room(&self, spec: &RoomSpec, ghost: &str, owner: &str) -> Result<String> {
        if let Some(room) = self.find_room(ghost, &spec.bridge_key).await? {
            return Ok(room);
        }
        self.create_room(spec, ghost, owner).await
    }

    pub async fn send_text(
        &self,
        room: &str,
        user: &str,
        transaction: &str,
        text: &str,
    ) -> Result<String> {
        ensure!(
            text.len() <= 48_000,
            "message exceeds Matrix text limit; delivery paused"
        );
        self.send_event(
            room,
            user,
            transaction,
            "m.room.message",
            json!({"msgtype": "m.text", "body": text}),
        )
        .await
    }

    pub async fn send_event(
        &self,
        room: &str,
        user: &str,
        transaction: &str,
        kind: &str,
        content: Value,
    ) -> Result<String> {
        let response = self
            .ok(
                Method::PUT,
                &["rooms", room, "send", kind, transaction],
                user,
                Some(content),
            )
            .await?;
        response["event_id"]
            .as_str()
            .context("Matrix send missing event_id")
            .map(str::to_string)
    }

    pub async fn event(&self, room: &str, user: &str, id: &str) -> Result<Value> {
        self.ok(Method::GET, &["rooms", room, "event", id], user, None)
            .await
    }

    pub async fn redact(&self, room: &str, user: &str, id: &str, transaction: &str) -> Result<()> {
        let (status, body) = self
            .request(
                Method::PUT,
                &["rooms", room, "redact", id, transaction],
                Some(user),
                Some(json!({})),
            )
            .await?;
        ensure!(
            status.is_success() || status == StatusCode::NOT_FOUND,
            "Matrix redact returned HTTP {status}: {body}"
        );
        Ok(())
    }

    /// Authenticated JSON request for bridge-specific endpoints. Callers get
    /// the status and body but never the token or the base URL.
    pub async fn request_json(
        &self,
        method: Method,
        path: &[&str],
        user: Option<&str>,
        body: Option<Value>,
    ) -> Result<(StatusCode, Value)> {
        self.request(method, path, user, body).await
    }

    pub async fn set_avatar(&self, user: &str, avatar_url: &str) -> Result<()> {
        self.ok(
            Method::PUT,
            &["profile", user, "avatar_url"],
            user,
            Some(json!({"avatar_url": avatar_url})),
        )
        .await?;
        Ok(())
    }

    pub async fn send_state(
        &self,
        room: &str,
        user: &str,
        kind: &str,
        state_key: &str,
        content: Value,
    ) -> Result<String> {
        let response = self
            .ok(
                Method::PUT,
                &["rooms", room, "state", kind, state_key],
                user,
                Some(content),
            )
            .await?;
        Ok(response["event_id"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    pub async fn room_messages(&self, room: &str, user: &str, from: Option<&str>) -> Result<Value> {
        let mut url = self.url(&["rooms", room, "messages"])?;
        url.query_pairs_mut()
            .append_pair("user_id", user)
            .append_pair("dir", "b")
            .append_pair("limit", "1000");
        if let Some(from) = from {
            url.query_pairs_mut().append_pair("from", from);
        }
        let response = self
            .client
            .get(url)
            .header(AUTHORIZATION, self.token.clone())
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Matrix history lookup failed: {}", e.without_url()))?;
        ensure!(
            response.status().is_success(),
            "Matrix history lookup returned HTTP {}",
            response.status()
        );
        Ok(response.json().await?)
    }

    /// MSC2716-style batch send, used by backfill.
    pub async fn batch_send(
        &self,
        room: &str,
        user: &str,
        events: Vec<Value>,
        forward: bool,
    ) -> Result<()> {
        let (status, body) = self
            .request(
                Method::POST,
                &["rooms", room, "batch_send"],
                Some(user),
                Some(json!({"events": events, "forward": forward})),
            )
            .await?;
        ensure!(
            status.is_success(),
            "Matrix batch_send returned HTTP {status}: {body}"
        );
        Ok(())
    }

    pub async fn room_state(&self, room: &str, user: &str) -> Result<Value> {
        self.ok(Method::GET, &["rooms", room, "state"], user, None)
            .await
    }

    /// Upload bytes to the Matrix media repository and return a validated mxc URI.
    pub async fn upload_media(
        &self,
        user: &str,
        name: &str,
        mime: &str,
        bytes: Vec<u8>,
    ) -> Result<String> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid Matrix base URL"))?
            .pop_if_empty()
            .extend(["_matrix", "media", "v3", "upload"]);
        url.query_pairs_mut()
            .append_pair("user_id", user)
            .append_pair("filename", name);
        let response = self
            .client
            .post(url)
            .header(AUTHORIZATION, self.token.clone())
            .header(reqwest::header::CONTENT_TYPE, mime)
            .body(bytes)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Matrix media upload failed: {}", e.without_url()))?;
        ensure!(
            response.status().is_success(),
            "Matrix media upload returned HTTP {}",
            response.status()
        );
        let result: Value = response.json().await?;
        let uri = result["content_uri"]
            .as_str()
            .context("Matrix upload missing content_uri")?;
        validate_mxc(uri)?;
        Ok(uri.to_string())
    }

    /// Download an `mxc://` attachment with a hard byte limit.
    pub async fn download_media(
        &self,
        user: &str,
        content: &Value,
        limit: usize,
    ) -> Result<Vec<u8>> {
        let uri = content
            .get("file")
            .and_then(|file| file.get("url"))
            .or_else(|| content.get("url"))
            .and_then(Value::as_str)
            .context("attachment is missing its Matrix media URI")?;
        let parsed = validate_mxc(uri)?;
        let server = parsed.host_str().context("mxc URI missing server")?;
        let id = parsed
            .path()
            .strip_prefix('/')
            .filter(|id| !id.is_empty() && !id.contains('/'))
            .context("invalid Matrix media identifier")?;
        let server = match parsed.port() {
            Some(port) => format!("{server}:{port}"),
            None => server.to_string(),
        };
        let mut url = self.base.clone();
        url.path_segments_mut()
            .unwrap()
            .pop_if_empty()
            .extend(["_matrix", "client", "v1", "media", "download", &server, id]);
        url.query_pairs_mut()
            .append_pair("user_id", user)
            .append_pair("allow_redirect", "false");
        let response = self
            .client
            .get(url)
            .header(AUTHORIZATION, self.token.clone())
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Matrix media download failed: {}", e.without_url()))?;
        ensure!(
            response.status().is_success(),
            "Matrix media download returned HTTP {}",
            response.status()
        );
        if let Some(size) = response.content_length() {
            ensure!(size <= limit as u64, "attachment exceeds the size limit");
        }
        let bytes = response.bytes().await?;
        ensure!(bytes.len() <= limit, "attachment exceeds the size limit");
        Ok(bytes.to_vec())
    }
}

/// A user-scoped Matrix client for the few operations an appservice cannot do
/// for a non-namespace user, such as accepting a room invite on the owner's
/// behalf. Keep this token in an owner-only secret file and make it opt-in.
#[derive(Clone)]
pub struct MatrixUser {
    client: Client,
    base: Url,
    token: HeaderValue,
}

impl MatrixUser {
    pub fn new(homeserver: &Url, token: impl AsRef<str>) -> Result<Self> {
        let mut token = HeaderValue::from_str(&format!("Bearer {}", token.as_ref()))?;
        token.set_sensitive(true);
        Ok(Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            base: homeserver.clone(),
            token,
        })
    }

    /// Join a room as the user. Matrix `/join` is idempotent, so this is safe
    /// to repeat when a bridge restarts.
    pub async fn join(&self, room: &str) -> Result<()> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid Matrix base URL"))?
            .pop_if_empty()
            .extend(["_matrix", "client", "v3", "join", room]);
        let response = self
            .client
            .post(url)
            .header(AUTHORIZATION, self.token.clone())
            .json(&json!({}))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Matrix join failed: {}", e.without_url()))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        ensure!(
            status.is_success() || status == StatusCode::CONFLICT,
            "Matrix join returned HTTP {status}: {body}"
        );
        Ok(())
    }
}

fn validate_mxc(uri: &str) -> Result<Url> {
    let parsed = Url::parse(uri)?;
    ensure!(
        parsed.scheme() == "mxc"
            && parsed.host_str().is_some()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.query().is_none()
            && parsed.fragment().is_none(),
        "invalid Matrix media URI"
    );
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> RegistrationBundle {
        serde_json::from_value(json!({
            "registration": {
                "id": "meshcore",
                "as_token": "a".repeat(32),
                "hs_token": "h".repeat(32),
                "sender_localpart": "meshcorebot",
                "namespaces": {"users": [{"regex": "@meshcore_.*", "exclusive": true}]}
            },
            "homeserver_url": "https://matrix.example",
            "homeserver_domain": "matrix.example",
            "your_user_id": "@owner:matrix.example"
        }))
        .unwrap()
    }

    #[test]
    fn ghosts_stay_inside_the_namespace() {
        let bundle = bundle();
        let matrix =
            MatrixAppService::new(&Url::parse("https://matrix.example").unwrap(), &bundle).unwrap();
        let ghost = matrix.ghost_for(&["tenant", "channel:0"]).unwrap();
        assert!(ghost.starts_with("@meshcore_"));
        assert!(ghost.ends_with(":matrix.example"));
    }

    #[test]
    fn ghosts_outside_the_namespace_are_rejected() {
        let mut bundle = bundle();
        bundle.registration.namespaces.users[0].regex = "@other_.*".into();
        let matrix =
            MatrixAppService::new(&Url::parse("https://matrix.example").unwrap(), &bundle).unwrap();
        assert!(matrix.ghost("abc").is_err());
    }

    #[test]
    fn transactions_skip_malformed_events() {
        let payload = json!({"events": [
            {"event_id": "$ok", "room_id": "!r", "sender": "@a:b", "type": "m.room.message",
             "content": {"msgtype": "m.text", "body": "hello"}, "origin_server_ts": 1_700_000_000_000u64},
            {"event_id": "$bad"},
        ]});
        let events = parse_transaction(&payload).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].text(), Some("hello"));
        assert_eq!(events[0].origin_server_ts, Some(1_700_000_000_000));
    }

    #[test]
    fn mxc_uris_are_strict() {
        assert!(validate_mxc("mxc://matrix.example/abc").is_ok());
        assert!(validate_mxc("mxc://matrix.example/abc?x=1").is_err());
        assert!(validate_mxc("https://matrix.example/abc").is_err());
    }
}
