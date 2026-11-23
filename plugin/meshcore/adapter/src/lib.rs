//! MeshCore Drawbridge adapter over loam:plugin/ipc.
//!
//! The adapter owns no Matrix or MeshCore state. It maps the typed Drawbridge
//! WIT calls to line-delimited IPC requests handled by `meshcore-host`.

wit_bindgen::generate!({
    path: "wit",
    world: "plugin",
    generate_all,
});

use crate::exports::ajmwagar::drawbridge::types::{
    Binding, Channel, ChannelSecret, Contact, Conversation, ConversationKind, Event, Health,
    MatrixEvent, Message, Page, SendAck, Transaction,
};
use crate::exports::ajmwagar::drawbridge::bridge::{Error, Guest};
use crate::loam::plugin::ipc;
use serde_json::{Value, json};

struct Component;

impl Guest for Component {
    fn poll(cursor: Option<String>, limit: u32) -> Result<Page, Error> {
        let value = request("poll", json!({"cursor": cursor, "limit": limit}))?;
        let events = value
            .get("events")
            .and_then(Value::as_array)
            .ok_or(Error::Invalid)?
            .iter()
            .map(event)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Page {
            events,
            next: value
                .get("next")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    fn ack(cursor: String) -> Result<(), Error> {
        request("ack", json!({"cursor": cursor})).map(|_| ())
    }

    fn send(conversation: Conversation, text: String) -> Result<SendAck, Error> {
        let value = request(
            "send",
            json!({"conversation": conversation_json(&conversation), "text": text}),
        )?;
        Ok(SendAck {
            request_id: string(&value, "request-id")
                .or_else(|_| string(&value, "request_id"))?,
            state: string(&value, "state")?,
        })
    }

    fn deliver(room: String, transaction_id: String, text: String) -> Result<String, Error> {
        let value = request(
            "deliver",
            json!({"room": room, "transaction": transaction_id, "text": text}),
        )?;
        Ok(value
            .get("event")
            .and_then(Value::as_str)
            .or_else(|| value.as_str())
            .ok_or(Error::Invalid)?
            .to_string())
    }

    fn ensure_room(
        conversation: Conversation,
        display: String,
        protocol: Option<String>,
    ) -> Result<Binding, Error> {
        let value = request(
            "ensure-room",
            json!({
                "conversation": conversation_json(&conversation),
                "display": display,
                "protocol": protocol.unwrap_or_else(|| "drawbridge".into()),
            }),
        )?;
        Ok(Binding {
            conversation,
            room: string(&value, "room")?,
            ghost: string(&value, "ghost")?,
            display_name: value
                .get("display")
                .and_then(Value::as_str)
                .unwrap_or(&display)
                .to_string(),
        })
    }

    fn bind(binding: Binding) -> Result<(), Error> {
        request(
            "bind",
            json!({
                "conversation": conversation_json(&binding.conversation),
                "room": binding.room,
                "ghost": binding.ghost,
                "display-name": binding.display_name,
            }),
        )
        .map(|_| ())
    }

    fn bindings() -> Result<Vec<Binding>, Error> {
        let value = request("bindings", json!({}))?;
        value
            .as_array()
            .ok_or(Error::Invalid)?
            .iter()
            .map(binding)
            .collect()
    }

    fn advert(flood: bool) -> Result<(), Error> {
        request("advert", json!({"flood": flood})).map(|_| ())
    }

    fn settings(
        name: Option<String>,
        tx_power: Option<i32>,
        path_hash_mode: Option<u8>,
        manual_add_contacts: Option<bool>,
    ) -> Result<(), Error> {
        request(
            "settings",
            json!({
                "name": name,
                "tx-power": tx_power,
                "path-hash-mode": path_hash_mode,
                "manual-add-contacts": manual_add_contacts,
            }),
        )
        .map(|_| ())
    }

    fn channels() -> Result<Vec<Channel>, Error> {
        let value = request("channels", json!({}))?;
        value
            .get("channels")
            .and_then(Value::as_array)
            .ok_or(Error::Invalid)?
            .iter()
            .map(|channel| {
                Ok(Channel {
                    index: channel
                        .get("index")
                        .and_then(Value::as_u64)
                        .ok_or(Error::Invalid)? as u8,
                    name: channel
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                })
            })
            .collect()
    }

    fn contacts() -> Result<Vec<Contact>, Error> {
        let value = request("contacts", json!({}))?;
        value
            .get("contacts")
            .and_then(Value::as_array)
            .ok_or(Error::Invalid)?
            .iter()
            .map(|contact| {
                Ok(Contact {
                    prefix: string(contact, "prefix")?,
                    name: contact
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    public_key: string(contact, "public-key")?,
                    chat: contact
                        .get("chat")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
            })
            .collect()
    }

    fn create_channel(name: String) -> Result<ChannelSecret, Error> {
        channel_key(request("channel.create", json!({"name": name}))?)
    }

    fn channel_key(index: u8) -> Result<ChannelSecret, Error> {
        channel_key(request("channel.key", json!({"index": index}))?)
    }

    fn claim(key: String) -> Result<bool, Error> {
        let value = request("claim", json!({"key": key}))?;
        value.as_bool().ok_or(Error::Invalid)
    }

    fn commit(transaction_id: String) -> Result<(), Error> {
        request("commit", json!({"transaction": transaction_id})).map(|_| ())
    }

    fn rollback(transaction_id: String) -> Result<(), Error> {
        request("rollback", json!({"transaction": transaction_id})).map(|_| ())
    }

    fn status() -> Result<Health, Error> {
        let value = request("status", json!({}))?;
        Ok(Health {
            sidecar_connected: value
                .get("connected")
                .and_then(Value::as_bool)
                .or_else(|| value.get("sidecar-connected").and_then(Value::as_bool))
                .unwrap_or(false),
            radio_name: string_or(&value, "name", "radio-name"),
            radio_public_key: string_or(&value, "public_key", "radio-public-key"),
            contacts: integer(&value, "contacts"),
            channels: integer(&value, "channels"),
        })
    }
}

fn channel_key(value: Value) -> Result<ChannelSecret, Error> {
    Ok(ChannelSecret {
        index: value
            .get("channel_idx")
            .or_else(|| value.get("index"))
            .and_then(Value::as_u64)
            .ok_or(Error::Invalid)? as u8,
        name: string(&value, "name")?,
        secret: string(&value, "secret")?,
        uri: string(&value, "uri")?,
    })
}

fn event(value: &Value) -> Result<Event, Error> {
    if let Some(message) = value.get("meshcore") {
        return Ok(Event::Meshcore(message_event(message)?));
    }
    if let Some(transaction) = value.get("matrix") {
        return Ok(Event::Matrix(transaction_event(transaction)?));
    }
    Err(Error::Invalid)
}

fn message_event(value: &Value) -> Result<Message, Error> {
    Ok(Message {
        id: string(value, "id")?,
        conversation: conversation(value.get("conversation").ok_or(Error::Invalid)?)?,
        sender: string(value, "sender")?,
        sender_name: value
            .get("sender-name")
            .and_then(Value::as_str)
            .map(str::to_string),
        text: string(value, "text")?,
        received_at_ms: value
            .get("received-at-ms")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

fn transaction_event(value: &Value) -> Result<Transaction, Error> {
    let events = value
        .get("events")
        .and_then(Value::as_array)
        .ok_or(Error::Invalid)?
        .iter()
        .map(matrix_event)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Transaction {
        id: string(value, "id")?,
        events,
    })
}

fn matrix_event(value: &Value) -> Result<MatrixEvent, Error> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .or_else(|| value.get("type").and_then(Value::as_str))
        .ok_or(Error::Invalid)?
        .to_string();
    let body = value
        .get("body")
        .and_then(Value::as_str)
        .or_else(|| value["content"]["body"].as_str())
        .map(str::to_string);
    Ok(MatrixEvent {
        event_id: value
            .get("event-id")
            .and_then(Value::as_str)
            .or_else(|| value.get("event_id").and_then(Value::as_str))
            .ok_or(Error::Invalid)?
            .to_string(),
        room: value
            .get("room")
            .and_then(Value::as_str)
            .or_else(|| value.get("room_id").and_then(Value::as_str))
            .ok_or(Error::Invalid)?
            .to_string(),
        sender: string(value, "sender")?,
        kind: kind.clone(),
        body,
        origin_server_ts_ms: value
            .get("origin-server-ts-ms")
            .and_then(Value::as_u64)
            .or_else(|| value.get("origin_server_ts").and_then(Value::as_u64)),
        encrypted: value
            .get("encrypted")
            .and_then(Value::as_bool)
            .unwrap_or(kind == "m.room.encrypted"),
    })
}

fn binding(value: &Value) -> Result<Binding, Error> {
    Ok(Binding {
        conversation: conversation_key(
            value
                .get("conversation")
                .or_else(|| value.get("key"))
                .ok_or(Error::Invalid)?,
        )?,
        room: string(value, "room")?,
        ghost: string(value, "ghost")?,
        display_name: value
            .get("display")
            .or_else(|| value.get("display-name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

fn conversation(value: &Value) -> Result<Conversation, Error> {
    let kind = value.get("kind").and_then(Value::as_str).ok_or(Error::Invalid)?;
    let id = value.get("id").and_then(Value::as_str).ok_or(Error::Invalid)?;
    Ok(Conversation {
        kind: match kind {
            "channel" => ConversationKind::Channel,
            "contact" => ConversationKind::Contact,
            _ => return Err(Error::Invalid),
        },
        id: id.to_string(),
    })
}

fn conversation_key(value: &Value) -> Result<Conversation, Error> {
    if value.is_object() {
        return conversation(value);
    }
    let key = value.as_str().ok_or(Error::Invalid)?;
    let (kind, id) = key.split_once(':').ok_or(Error::Invalid)?;
    Ok(Conversation {
        kind: match kind {
            "channel" => ConversationKind::Channel,
            "contact" => ConversationKind::Contact,
            _ => return Err(Error::Invalid),
        },
        id: id.to_string(),
    })
}

fn conversation_json(conversation: &Conversation) -> Value {
    json!({
        "kind": match conversation.kind {
            ConversationKind::Channel => "channel",
            ConversationKind::Contact => "contact",
        },
        "id": conversation.id,
    })
}

fn request(method: &str, payload: Value) -> Result<Value, Error> {
    let bytes = serde_json::to_vec(&payload).map_err(|_| Error::Invalid)?;
    let result = ipc::request(method, &bytes).map_err(map_error)?;
    serde_json::from_slice(&result).map_err(|_| Error::Invalid)
}

fn map_error(error: ipc::Error) -> Error {
    match error {
        ipc::Error::Denied => Error::Denied,
        ipc::Error::Unavailable => Error::Unavailable,
        ipc::Error::Invalid => Error::Invalid,
        ipc::Error::Io => Error::Io,
    }
}

fn string(value: &Value, key: &str) -> Result<String, Error> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or(Error::Invalid)
}

fn string_or(value: &Value, first: &str, second: &str) -> String {
    value
        .get(first)
        .and_then(Value::as_str)
        .or_else(|| value.get(second).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn integer(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

export!(Component);
