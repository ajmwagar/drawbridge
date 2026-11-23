//! MeshCore host sidecar for the Drawbridge Loam plugin.
//!
//! Reads line-delimited JSON on stdin, writes responses on stdout. The Matrix
//! appservice and room delivery are the next slice; this binary currently
//! exposes MeshCore status, polling, sends, adverts, settings and channel
//! management plus the durable claim/binding surface.

use anyhow::{Context, Result};
use drawbridge_core::secret;
use drawbridge_host::{ipc, meshcore::MeshCoreBackend};
use drawbridge_meshcore::MeshCoreClient;
use drawbridge_store::Store;
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use url::Url;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostConfig {
    database: PathBuf,
    encryption_key_env: String,
    #[serde(default = "tenant_default")]
    tenant: String,
    #[serde(default)]
    backfill: bool,
    meshcore: MeshCoreConfig,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeshCoreConfig {
    url: Url,
    token_env: String,
}

fn tenant_default() -> String {
    "personal".into()
}

impl HostConfig {
    fn load(path: &Path) -> Result<Self> {
        let mut config: Self = toml::from_str(&std::fs::read_to_string(path)?)
            .context("invalid drawbridge host configuration")?;
        let root = path.parent().unwrap_or(Path::new("."));
        if config.database.is_relative() {
            config.database = root.join(&config.database);
        }
        Ok(config)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let mut args = std::env::args().skip(1);
    let mut config_path = PathBuf::from("drawbridge-host.toml");
    while let Some(arg) = args.next() {
        if arg == "--config" {
            config_path = args.next().context("--config requires a path")?.into();
        }
    }
    let config = HostConfig::load(&config_path)?;
    let database_key = secret(&config.encryption_key_env)?;
    let store = Arc::new(Store::open(&config.database, &database_key, "")?);
    let token = secret(&config.meshcore.token_env)?;
    let client = MeshCoreClient::new(config.meshcore.url, token)?;
    if !config.backfill
        && store.cursor(&config.tenant, "meshcore")? == 0
        && let Ok(latest) = client.latest_id().await
        && latest > 0
    {
        store.set_cursor(&config.tenant, "meshcore", latest)?;
    }
    let backend = Arc::new(MeshCoreBackend::new(client, store, config.tenant));
    ipc::serve(backend, tokio::io::stdin(), tokio::io::stdout()).await
}
