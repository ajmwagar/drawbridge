//! Durable, idempotent SQLite storage for Drawbridge bridges.
//!
//! Extracted from `FuturePresentLabs/rakazo-beeper`'s `src/store.rs`: the
//! single-process lock, WAL mode, key check and XChaCha20-Poly1305 payload
//! encryption are kept. The Rakazo-specific tables are replaced with a small
//! generic schema (bindings, links, cursors, transactions, events) that a bridge
//! can extend with its own DDL.

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use fs2::FileExt;
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    path::Path,
    sync::Mutex,
};

const KEY_CHECK: &str = "drawbridge-store-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub tenant: String,
    /// Network-native conversation key, e.g. `channel:0` or `contact:abc123`.
    pub key: String,
    pub room: String,
    pub ghost: String,
    pub display: String,
}

pub struct Store {
    db: Mutex<Connection>,
    cipher: XChaCha20Poly1305,
    _lock: DatabaseLock,
}

struct DatabaseLock(File);
impl Drop for DatabaseLock {
    fn drop(&mut self) {
        if let Err(error) = FileExt::unlock(&self.0) {
            tracing::error!(%error, "could not release database process lock");
        }
    }
}

impl Store {
    /// Open a bridge database.
    ///
    /// `key_b64` must decode to 32 bytes. `schema` is the application's DDL; it
    /// runs after the generic schema and must be idempotent. The database is
    /// created with owner-only permissions and guarded by an exclusive process
    /// lock, because two bridges writing one SQLite file is never a valid state.
    pub fn open(path: &Path, key_b64: &str, schema: &str) -> Result<Self> {
        let decoded = STANDARD
            .decode(key_b64)
            .context("database key must be base64")?;
        ensure!(decoded.len() == 32, "database key must decode to 32 bytes");
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(path.with_extension("lockfile"))?;
        lock.try_lock_exclusive()
            .context("another bridge process owns this database")?;
        let lock = DatabaseLock(lock);
        let _ = options.open(path)?;
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value BLOB NOT NULL);
             CREATE TABLE IF NOT EXISTS transactions (txn TEXT PRIMARY KEY, seen_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS events (tenant TEXT NOT NULL, event_id TEXT NOT NULL, seen_at INTEGER NOT NULL, PRIMARY KEY(tenant,event_id));
             CREATE TABLE IF NOT EXISTS bindings (
               tenant TEXT NOT NULL,
               key TEXT NOT NULL,
               room TEXT NOT NULL,
               ghost TEXT NOT NULL,
               display TEXT NOT NULL,
               PRIMARY KEY(tenant,key),
               UNIQUE(tenant,room)
             );
             CREATE TABLE IF NOT EXISTS links (
               tenant TEXT NOT NULL,
               local_id TEXT NOT NULL,
               matrix_id TEXT NOT NULL,
               direction TEXT NOT NULL,
               conversation TEXT NOT NULL,
               part TEXT NOT NULL DEFAULT 'body',
               created_at INTEGER NOT NULL,
               PRIMARY KEY(tenant,local_id,matrix_id,part)
             );
             CREATE INDEX IF NOT EXISTS links_local ON links(tenant,local_id);
             CREATE INDEX IF NOT EXISTS links_matrix ON links(tenant,matrix_id);
             CREATE TABLE IF NOT EXISTS cursors (
               tenant TEXT NOT NULL,
               kind TEXT NOT NULL,
               cursor INTEGER NOT NULL,
               PRIMARY KEY(tenant,kind)
             );
             CREATE TABLE IF NOT EXISTS inbound (
               id INTEGER PRIMARY KEY,
               tenant TEXT NOT NULL,
               event_id TEXT NOT NULL,
               conversation TEXT NOT NULL,
               payload BLOB,
               state TEXT NOT NULL DEFAULT 'pending',
               attempts INTEGER NOT NULL DEFAULT 0,
               next_at INTEGER NOT NULL DEFAULT 0,
               error TEXT,
               UNIQUE(tenant,event_id)
             );
             CREATE INDEX IF NOT EXISTS inbound_pending ON inbound(tenant,state,id);",
        )?;
        if !schema.trim().is_empty() {
            db.execute_batch(schema)
                .context("application schema failed")?;
        }
        let store = Self {
            db: Mutex::new(db),
            cipher: XChaCha20Poly1305::new_from_slice(&decoded).unwrap(),
            _lock: lock,
        };
        let saved: Option<Vec<u8>> = store
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM metadata WHERE key='key-check'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(saved) = saved {
            ensure!(
                store.decrypt("key-check", &saved)? == KEY_CHECK.as_bytes(),
                "incorrect database key"
            );
        } else {
            store.db.lock().unwrap().execute(
                "INSERT INTO metadata VALUES ('key-check',?1)",
                [store.encrypt("key-check", KEY_CHECK.as_bytes())?],
            )?;
        }
        Ok(store)
    }

    pub fn encrypt(&self, context: &str, bytes: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut nonce);
        let mut out = nonce.to_vec();
        out.extend(
            self.cipher
                .encrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: bytes,
                        aad: context.as_bytes(),
                    },
                )
                .map_err(|_| anyhow::anyhow!("encrypt failed"))?,
        );
        Ok(out)
    }

    pub fn decrypt(&self, context: &str, bytes: &[u8]) -> Result<Vec<u8>> {
        ensure!(bytes.len() > 24, "ciphertext too short");
        self.cipher
            .decrypt(
                XNonce::from_slice(&bytes[..24]),
                Payload {
                    msg: &bytes[24..],
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("decrypt failed"))
    }

    /// Claim a Matrix transaction exactly once. Returns `false` if it was
    /// already acknowledged.
    pub fn claim_transaction(&self, txn: &str) -> Result<bool> {
        Ok(self.db.lock().unwrap().execute(
            "INSERT OR IGNORE INTO transactions VALUES(?1,?2)",
            params![txn, drawbridge_core::now()],
        )? > 0)
    }

    /// Claim a network event exactly once. Returns `false` if it was already
    /// processed.
    pub fn claim_event(&self, tenant: &str, event_id: &str) -> Result<bool> {
        Ok(self.db.lock().unwrap().execute(
            "INSERT OR IGNORE INTO events VALUES(?1,?2,?3)",
            params![tenant, event_id, drawbridge_core::now()],
        )? > 0)
    }

    pub fn bindings(&self, tenant: &str) -> Result<Vec<Binding>> {
        let db = self.db.lock().unwrap();
        let mut query = db.prepare(
            "SELECT tenant,key,room,ghost,display FROM bindings WHERE tenant=?1 ORDER BY key",
        )?;
        Ok(query
            .query_map([tenant], |row| {
                Ok(Binding {
                    tenant: row.get(0)?,
                    key: row.get(1)?,
                    room: row.get(2)?,
                    ghost: row.get(3)?,
                    display: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn binding(&self, tenant: &str, key: &str) -> Result<Option<Binding>> {
        let db = self.db.lock().unwrap();
        Ok(db
            .query_row(
                "SELECT tenant,key,room,ghost,display FROM bindings WHERE tenant=?1 AND key=?2",
                params![tenant, key],
                |row| {
                    Ok(Binding {
                        tenant: row.get(0)?,
                        key: row.get(1)?,
                        room: row.get(2)?,
                        ghost: row.get(3)?,
                        display: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn binding_by_room(&self, tenant: &str, room: &str) -> Result<Option<Binding>> {
        let db = self.db.lock().unwrap();
        Ok(db
            .query_row(
                "SELECT tenant,key,room,ghost,display FROM bindings WHERE tenant=?1 AND room=?2",
                params![tenant, room],
                |row| {
                    Ok(Binding {
                        tenant: row.get(0)?,
                        key: row.get(1)?,
                        room: row.get(2)?,
                        ghost: row.get(3)?,
                        display: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn upsert_binding(&self, binding: &Binding) -> Result<()> {
        self.db.lock().unwrap().execute(
            "INSERT INTO bindings VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(tenant,key) DO UPDATE SET room=excluded.room,ghost=excluded.ghost,display=excluded.display",
            params![binding.tenant, binding.key, binding.room, binding.ghost, binding.display],
        )?;
        Ok(())
    }

    pub fn cursor(&self, tenant: &str, kind: &str) -> Result<i64> {
        Ok(self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT cursor FROM cursors WHERE tenant=?1 AND kind=?2",
                params![tenant, kind],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    pub fn set_cursor(&self, tenant: &str, kind: &str, cursor: i64) -> Result<()> {
        self.db.lock().unwrap().execute(
            "INSERT INTO cursors VALUES(?1,?2,?3)
             ON CONFLICT(tenant,kind) DO UPDATE SET cursor=excluded.cursor",
            params![tenant, kind, cursor],
        )?;
        Ok(())
    }

    /// Record the link between a network message and a Matrix event. Returns
    /// `false` when the link already existed.
    pub fn link(
        &self,
        tenant: &str,
        local_id: &str,
        matrix_id: &str,
        direction: &str,
        conversation: &str,
    ) -> Result<bool> {
        Ok(self.db.lock().unwrap().execute(
            "INSERT OR IGNORE INTO links VALUES(?1,?2,?3,?4,?5,'body',?6)",
            params![
                tenant,
                local_id,
                matrix_id,
                direction,
                conversation,
                drawbridge_core::now()
            ],
        )? > 0)
    }

    pub fn linked(&self, tenant: &str, local_id: &str) -> Result<bool> {
        Ok(self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT 1 FROM links WHERE tenant=?1 AND local_id=?2 LIMIT 1",
                params![tenant, local_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn pending_count(&self) -> Result<u64> {
        Ok(self.db.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM inbound WHERE state='pending'",
            [],
            |r| r.get(0),
        )?)
    }

    /// Run an application-specific query under the database lock.
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        f(&self.db.lock().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    fn key() -> String {
        STANDARD.encode([7u8; 32])
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("bridge.db"), &key(), "").unwrap();
        (dir, store)
    }

    #[test]
    fn transactions_and_events_are_claimed_once() {
        let (_dir, store) = store();
        assert!(store.claim_transaction("txn-1").unwrap());
        assert!(!store.claim_transaction("txn-1").unwrap());
        assert!(store.claim_event("tenant", "$event").unwrap());
        assert!(!store.claim_event("tenant", "$event").unwrap());
    }

    #[test]
    fn bindings_are_unique_per_room_and_key() {
        let (_dir, store) = store();
        let binding = Binding {
            tenant: "t".into(),
            key: "channel:0".into(),
            room: "!room:t".into(),
            ghost: "@meshcore_x:t".into(),
            display: "Public".into(),
        };
        store.upsert_binding(&binding).unwrap();
        assert_eq!(
            store.binding("t", "channel:0").unwrap(),
            Some(binding.clone())
        );
        assert_eq!(
            store.binding_by_room("t", "!room:t").unwrap(),
            Some(binding)
        );
        let duplicate_room = Binding {
            tenant: "t".into(),
            key: "channel:1".into(),
            room: "!room:t".into(),
            ghost: "@meshcore_y:t".into(),
            display: "Other".into(),
        };
        assert!(store.upsert_binding(&duplicate_room).is_err());
    }

    #[test]
    fn cursors_round_trip_and_links_are_idempotent() {
        let (_dir, store) = store();
        assert_eq!(store.cursor("t", "meshcore").unwrap(), 0);
        store.set_cursor("t", "meshcore", 42).unwrap();
        assert_eq!(store.cursor("t", "meshcore").unwrap(), 42);
        assert!(
            store
                .link("t", "m1", "$e1", "inbound", "channel:0")
                .unwrap()
        );
        assert!(
            !store
                .link("t", "m1", "$e1", "inbound", "channel:0")
                .unwrap()
        );
        assert!(store.linked("t", "m1").unwrap());
    }

    #[test]
    fn encrypted_payloads_round_trip_and_reject_other_contexts() {
        let (_dir, store) = store();
        let ciphertext = store.encrypt("payload", b"secret").unwrap();
        assert_eq!(store.decrypt("payload", &ciphertext).unwrap(), b"secret");
        assert!(store.decrypt("other", &ciphertext).is_err());
    }

    #[test]
    fn a_second_process_cannot_open_the_same_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.db");
        let _first = Store::open(&path, &key(), "").unwrap();
        assert!(Store::open(&path, &key(), "").is_err());
    }
}
