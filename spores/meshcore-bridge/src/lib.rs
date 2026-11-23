//! Soil policy for the MeshCore bridge.
//!
//! The loop is deliberately small: poll one page, route each event, ack the
//! cursor only after every effect is durable. Matrix delivery uses the network
//! message id as its transaction id, so a crash between delivery and `ack`
//! replays without creating a second room event.

wit_bindgen::generate!({
    path: "wit",
    world: "bridge",
    generate_all,
});

use crate::ajmwagar::drawbridge::bridge;
use crate::exports::wasi::cli::run::Guest as CliGuest;
use crate::ajmwagar::drawbridge::types::{
    Conversation, ConversationKind, Event, Health, Message, Transaction,
};
use std::thread::sleep;
use std::time::Duration;

const HELP: &str = "\
MeshCore commands (owner only):
/mesh status
/mesh help

The Rust fallback still owns advert, name, txpower, pathmode, autoadd,
channels, contacts and channel key/create until those host calls move into
the Drawbridge capability.";

struct Component;

export!(Component);

impl Guest for Component {
    fn run(args: Vec<String>) -> Result<String, String> {
        run_loop(&args)
    }
}

impl CliGuest for Component {
    fn run() -> Result<(), ()> {
        let mut args = crate::wasi::cli::environment::get_arguments();
        if !args.is_empty() {
            args.remove(0);
        }
        run_loop(&args).map(|_| ()).map_err(|error| {
            eprintln!("meshcore-bridge: {error}");
        })
    }
}

fn run_loop(args: &[String]) -> Result<String, String> {
    let options = Options::parse(args)?;
    let mut cursor = options.cursor.clone();
    loop {
        let page = match bridge::poll(cursor.as_deref(), options.limit) {
            Ok(page) => page,
            Err(error) => {
                if options.once {
                    return Err(format!("poll failed: {}", format_error(error)));
                }
                sleep(options.interval);
                continue;
            }
        };

        for event in page.events {
            let result = match event {
                Event::Meshcore(message) => handle_meshcore(message),
                Event::Matrix(transaction) => handle_matrix(&options.owner, transaction),
            };
            if let Err(error) = result {
                if options.once {
                    return Err(error);
                }
                // The cursor is not acked, so the page is retried. Delivery is
                // idempotent on the network message id.
                sleep(options.interval);
                break;
            }
        }

        if let Some(next) = page.next {
            if cursor.as_deref() != Some(next.as_str()) {
                if let Err(error) = bridge::ack(&next) {
                    if options.once {
                        return Err(format!("ack failed: {}", format_error(error)));
                    }
                    sleep(options.interval);
                    continue;
                }
                cursor = Some(next);
            }
        }

        if options.once {
            return Ok("processed one page".into());
        }
        sleep(options.interval);
    }
}

fn handle_meshcore(message: Message) -> Result<(), String> {
    if message.text.trim().is_empty() {
        return Ok(());
    }
    let display = display_for(&message.conversation, message.sender_name.as_deref());
    let binding = bridge::ensure_room(
        &message.conversation,
        &display,
        Some("meshcore"),
    )
    .map_err(format_error)?;
    let transaction = format!("meshcore:{}", message.id);
    bridge::deliver(&binding.room, &transaction, &message.text).map_err(format_error)?;
    Ok(())
}

fn handle_matrix(owner: &str, transaction: Transaction) -> Result<(), String> {
    let bindings = bridge::bindings().map_err(format_error)?;
    for event in &transaction.events {
        if event.encrypted {
            continue;
        }
        if event.sender != owner {
            continue;
        }
        if event.kind != "m.room.message" {
            continue;
        }
        let Some(text) = event.body.as_deref().map(str::trim).filter(|text| !text.is_empty())
        else {
            continue;
        };
        let Some(binding) = bindings.iter().find(|binding| binding.room == event.room) else {
            continue;
        };

        if is_command(text) {
            let response = command(owner, &binding.conversation, text);
            let transaction = format!("command:{}", event.event_id);
            bridge::deliver(&binding.room, &transaction, &response)
                .map_err(format_error)?;
            continue;
        }

        bridge::send(&binding.conversation, text).map_err(|error| {
            format!(
                "send {} to MeshCore failed: {}",
                binding.conversation.id,
                format_error(error)
            )
        })?;
    }
    bridge::commit(&transaction.id).map_err(format_error)?;
    Ok(())
}

fn is_command(text: &str) -> bool {
    let text = text.trim();
    text == "/mesh"
        || text.starts_with("/mesh ")
        || text == "!mesh"
        || text.starts_with("!mesh ")
}

fn command(owner: &str, _conversation: &Conversation, text: &str) -> String {
    let text = text.trim();
    let rest = text
        .strip_prefix("/mesh")
        .or_else(|| text.strip_prefix("!mesh"))
        .unwrap_or("")
        .trim();
    match rest {
        "" | "help" => HELP.to_string(),
        "status" => match bridge::status() {
            Ok(health) => status_line(&health),
            Err(error) => format!("Command failed: {}", format_error(error)),
        },
        other => format!(
            "`/mesh {other}` is not available in the Spore yet. The Rust fallback still owns it.\nOwner: {owner}"
        ),
    }
}

fn status_line(health: &Health) -> String {
    let key = if health.radio_public_key.len() > 12 {
        &health.radio_public_key[..12]
    } else {
        &health.radio_public_key
    };
    format!(
        "{} · {} · contacts {} · channels {}",
        health.radio_name, key, health.contacts, health.channels
    )
}

fn display_for(conversation: &Conversation, sender_name: Option<&str>) -> String {
    match conversation.kind {
        ConversationKind::Channel if conversation.id == "0" => "Public".into(),
        ConversationKind::Channel => format!("MeshCore #{}", conversation.id),
        ConversationKind::Contact => sender_name
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                let prefix = conversation.id.chars().take(8).collect::<String>();
                format!("MeshCore DM {prefix}")
            }),
    }
}

fn format_error(error: bridge::Error) -> String {
    match error {
        bridge::Error::Denied => "denied".into(),
        bridge::Error::Invalid => "invalid request".into(),
        bridge::Error::Unavailable => "host unavailable".into(),
        bridge::Error::OverBudget => "over budget".into(),
        bridge::Error::Conflict => "conflict".into(),
        bridge::Error::Stale => "stale cursor".into(),
        bridge::Error::Unsupported => "unsupported".into(),
        bridge::Error::Io => "host I/O error".into(),
    }
}

#[derive(Debug)]
struct Options {
    owner: String,
    limit: u32,
    interval: Duration,
    once: bool,
    cursor: Option<String>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut owner = None;
        let mut limit = 100u32;
        let mut interval_ms = 2_000u64;
        let mut once = false;
        let mut cursor = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--owner" => {
                    owner = Some(
                        args.get(i + 1)
                            .ok_or("--owner needs a Matrix user id")?
                            .clone(),
                    );
                    i += 2;
                }
                "--limit" => {
                    limit = args
                        .get(i + 1)
                        .ok_or("--limit needs a number")?
                        .parse()
                        .map_err(|_| "--limit must be a number")?;
                    i += 2;
                }
                "--interval-ms" => {
                    interval_ms = args
                        .get(i + 1)
                        .ok_or("--interval-ms needs a number")?
                        .parse()
                        .map_err(|_| "--interval-ms must be a number")?;
                    i += 2;
                }
                "--once" => {
                    once = true;
                    i += 1;
                }
                "--cursor" => {
                    cursor = Some(
                        args.get(i + 1)
                            .ok_or("--cursor needs a message id")?
                            .clone(),
                    );
                    i += 2;
                }
                other => return Err(format!("unknown option {other}")),
            }
        }
        let owner = owner.ok_or("--owner @user:server is required")?;
        if !owner.starts_with('@') || !owner.contains(':') {
            return Err("--owner must be a Matrix user id".into());
        }
        Ok(Self {
            owner,
            limit: limit.clamp(1, 500),
            interval: Duration::from_millis(interval_ms.max(100)),
            once,
            cursor,
        })
    }
}
