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
    Channel, Conversation, ConversationKind, Event, Health, Message, Transaction,
};
use std::thread::sleep;
use std::time::Duration;

const HELP: &str = "\
MeshCore commands (owner only):
/mesh status
/mesh advert [flood|zerohop]
/mesh name <name>
/mesh txpower <0-30>
/mesh pathmode <0|1|2>
/mesh autoadd on|off
/mesh channels
/mesh contacts
/mesh channel create <name>
/mesh channel key <index>
/mesh channel share <index>";

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
    match run_command(text) {
        Ok(response) => response,
        Err(error) => format!("Command failed: {error}\nOwner: {owner}"),
    }
}

fn run_command(text: &str) -> Result<String, String> {
    let text = text.trim();
    let rest = text
        .strip_prefix("/mesh")
        .or_else(|| text.strip_prefix("!mesh"))
        .unwrap_or("")
        .trim();
    let mut parts = rest.split_whitespace();
    let word = parts.next().unwrap_or("help").to_ascii_lowercase();
    let args: Vec<&str> = parts.collect();
    match word.as_str() {
        "" | "help" => Ok(HELP.to_string()),
        "status" => {
            let health = bridge::status().map_err(format_error)?;
            Ok(status_line(&health))
        }
        "advert" => {
            let flood = match args.as_slice() {
                [] | ["flood"] => true,
                ["zerohop"] | ["zero-hop"] | ["zero"] => false,
                _ => return Err("usage: /mesh advert [flood|zerohop]".into()),
            };
            bridge::advert(flood).map_err(format_error)?;
            Ok(if flood {
                "Flood advert sent.".into()
            } else {
                "Zero-hop advert sent.".into()
            })
        }
        "name" => {
            let name = args.join(" ");
            validate_name(&name)?;
            bridge::settings(Some(&name), None, None, None).map_err(format_error)?;
            Ok(format!("Radio name set to {name}."))
        }
        "txpower" | "tx-power" => {
            let value = parse_i32(args.first(), "usage: /mesh txpower <0-30>")?;
            if !(0..=30).contains(&value) {
                return Err("txpower must be 0-30".into());
            }
            bridge::settings(None, Some(value), None, None).map_err(format_error)?;
            Ok(format!("TX power set to {value} dBm."))
        }
        "pathmode" | "path-mode" => {
            let value = parse_u8(args.first(), "usage: /mesh pathmode <0|1|2>")?;
            if value > 2 {
                return Err("pathmode must be 0, 1 or 2".into());
            }
            bridge::settings(None, None, Some(value), None).map_err(format_error)?;
            Ok(format!(
                "Message path hash mode set to {value} ({} bytes).",
                value + 1
            ))
        }
        "autoadd" | "auto-add" => {
            let enabled = match args.as_slice() {
                ["on"] | ["true"] | ["yes"] => true,
                ["off"] | ["false"] | ["no"] => false,
                _ => return Err("usage: /mesh autoadd on|off".into()),
            };
            bridge::settings(None, None, None, Some(!enabled)).map_err(format_error)?;
            Ok(format!(
                "Contact auto-add {}.",
                if enabled {
                    "enabled"
                } else {
                    "disabled (manual add only)"
                }
            ))
        }
        "channels" => {
            let channels = bridge::channels().map_err(format_error)?;
            Ok(format_channels(&channels))
        }
        "contacts" => {
            let contacts = bridge::contacts().map_err(format_error)?;
            let chats = contacts.iter().filter(|contact| contact.chat).count();
            Ok(format!(
                "{} contacts on the radio; {chats} chat contacts.",
                contacts.len()
            ))
        }
        "channel" => match args.as_slice() {
            ["create", rest @ ..] => {
                let name = rest.join(" ");
                validate_channel_name(&name)?;
                let created = bridge::create_channel(&name).map_err(format_error)?;
                let conversation = Conversation {
                    kind: ConversationKind::Channel,
                    id: created.index.to_string(),
                };
                let room = match bridge::ensure_room(
                    &conversation,
                    &created.name,
                    Some("meshcore"),
                ) {
                    Ok(binding) => binding.room,
                    Err(error) => format!("(room creation failed: {})", format_error(error)),
                };
                Ok(format!(
                    "Created private channel #{} '{}'.\nKey: {}\nShare: {}\nRoom: {}\nAnyone with this key can read the channel.",
                    created.index, created.name, created.secret, created.uri, room
                ))
            }
            ["key", index] => {
                let index = parse_u8(Some(index), "usage: /mesh channel key <index>")?;
                let channel = bridge::channel_key(index).map_err(format_error)?;
                Ok(format!(
                    "Channel #{} '{}'.\nKey: {}\nShare: {}\nTreat this key as a secret.",
                    channel.index, channel.name, channel.secret, channel.uri
                ))
            }
            ["share", index] => {
                let index = parse_u8(Some(index), "usage: /mesh channel share <index>")?;
                let channel = bridge::channel_key(index).map_err(format_error)?;
                Ok(format!(
                    "Channel #{} '{}': {}",
                    channel.index, channel.name, channel.uri
                ))
            }
            _ => Err("usage: /mesh channel create <name> | key <index> | share <index>".into()),
        },
        other => Err(format!("unknown command {other}; try /mesh help")),
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        Err("usage: /mesh name <name>".into())
    } else if name.len() > 32 || name.chars().any(|character| !character.is_ascii() || character.is_ascii_control()) {
        Err("name must be 1-32 printable ASCII characters".into())
    } else {
        Ok(())
    }
}

fn validate_channel_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        Err("usage: /mesh channel create <name>".into())
    } else if name.starts_with('#') {
        Err("private channel names must not start with #".into())
    } else if name.len() > 32 || name.chars().any(|character| !character.is_ascii() || character.is_ascii_control()) {
        Err("name must be 1-32 printable ASCII characters".into())
    } else {
        Ok(())
    }
}

fn parse_i32(value: Option<&&str>, usage: &str) -> Result<i32, String> {
    value
        .ok_or_else(|| usage.to_string())?
        .parse()
        .map_err(|_| usage.to_string())
}

fn parse_u8(value: Option<&&str>, usage: &str) -> Result<u8, String> {
    value
        .ok_or_else(|| usage.to_string())?
        .parse()
        .map_err(|_| usage.to_string())
}

fn format_channels(channels: &[Channel]) -> String {
    if channels.is_empty() {
        return "No channels.".into();
    }
    let names = channels
        .iter()
        .map(|channel| {
            if channel.name.is_empty() {
                format!("#{}", channel.index)
            } else {
                format!("{} (#{})", channel.name, channel.index)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("Channels: {names}")
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
