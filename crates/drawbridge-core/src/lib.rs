//! Transport-neutral primitives shared by Drawbridge bridges.
//!
//! This crate deliberately knows nothing about Matrix or any chat network. It
//! provides the small set of invariants every bridge needs: stable identifiers,
//! secret loading, wall-clock time, and the normalized inbound-event shape.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// Length-delimited hashing keeps tenant and remote identifiers unambiguous.
///
/// The same input always produces the same identifier, which makes ghost IDs,
/// Matrix transaction IDs and deterministic retries safe across restarts.
pub fn stable_id(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    hex::encode(hash.finalize())
}

/// A short, human-scannable form of [`stable_id`].
pub fn short_id(parts: &[&str], bytes: usize) -> String {
    stable_id(parts)[..bytes.min(64)].to_string()
}

/// Unix seconds.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Read a secret from `NAME` or, preferably, from the file named by
/// `NAME_FILE`.
///
/// A secret file must not be accessible to group or other users. This mirrors
/// the deployment rule used by the existing bridges: secrets live in files owned
/// by the bridge user, not in the process environment or a repository.
pub fn secret(name: &str) -> Result<String> {
    let file_name = format!("{name}_FILE");
    let direct = std::env::var(name).ok();
    let file = std::env::var(&file_name).ok();
    let value = match (direct, file) {
        (Some(value), None) => value,
        (None, Some(path)) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                ensure!(
                    std::fs::metadata(&path)?.permissions().mode() & 0o077 == 0,
                    "secret file referenced by {file_name} must be accessible only to its owner"
                );
            }
            std::fs::read_to_string(&path)
                .with_context(|| format!("read secret file {path}"))?
                .trim()
                .to_string()
        }
        (Some(_), Some(_)) => {
            anyhow::bail!("set exactly one of {name} or {file_name}, not both")
        }
        (None, None) => anyhow::bail!("missing secret: set {name} or {file_name}"),
    };
    ensure!(!value.is_empty(), "secret {name} is empty");
    Ok(value)
}

/// The normalized shape every network adapter produces before a bridge
/// decides whether and where to deliver it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundEvent {
    /// Network-native identifier. It must be stable for retries.
    pub remote_id: String,
    /// Network-native conversation identifier (`channel:0`, `contact:abc`, …).
    pub conversation: String,
    /// Network-native sender identifier.
    pub sender: String,
    /// Human-readable sender name when the network exposes one.
    pub sender_name: Option<String>,
    /// Message body as plain text.
    pub body: String,
    /// Unix seconds from the network, if known.
    pub sent_at: Option<u64>,
}

/// A room this bridge wants to own. The matrix crate turns this into state
/// events; the network adapter supplies the protocol identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomSpec {
    pub name: String,
    pub topic: String,
    /// Stable bridge key, e.g. `meshcore://tenant/channel:0`.
    pub bridge_key: String,
    pub protocol_id: String,
    pub protocol_display_name: String,
    pub channel_id: String,
    pub channel_display_name: String,
}

/// How a bridge identifies itself in `m.bridge`/`uk.half-shot.bridge` state.
pub mod protocol {
    pub const MESHCORE: &str = "meshcore";
    pub const MESHCORE_NAME: &str = "MeshCore";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_ids_are_unambiguous() {
        assert_ne!(stable_id(&["ab", "c"]), stable_id(&["a", "bc"]));
        assert_eq!(stable_id(&["ab", "c"]), stable_id(&["ab", "c"]));
        assert_eq!(stable_id(&["x"]).len(), 64);
    }

    #[test]
    fn short_ids_are_prefixes() {
        assert_eq!(short_id(&["room"], 12), stable_id(&["room"])[..12]);
    }
}
