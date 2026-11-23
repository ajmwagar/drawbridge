//! Rakazo host sidecar for the Drawbridge Loam plugin.
//!
//! Line-delimited JSON on stdin/stdout. This slice covers status, bots, polling,
//! assistant sends and the durable claim/binding surface. Matrix delivery is the
//! next slice.

use anyhow::{Context, Result};
use drawbridge_core::secret;
use drawbridge_host::{ipc, rakazo::RakazoBackend};
use drawbridge_rakazo::RakazoClient;
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
    bots: Vec<String>,
    rakazo: RakazoConfig,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RakazoConfig {
    url: Url,
    token_env: String,
    space_id_env: String,
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
    let mut config_path = PathBuf::from("rakazo-host.toml");
    while let Some(arg) = args.next() {
        if arg == "--config" {
            config_path = args.next().context("--config requires a path")?.into();
        }
    }
    let config = HostConfig::load(&config_path)?;
    let database_key = secret(&config.encryption_key_env)?;
    let store = Arc::new(Store::open(&config.database, &database_key, "")?);
    let token = secret(&config.rakazo.token_env)?;
    let space = secret(&config.rakazo.space_id_env)?;
    let client = RakazoClient::new(config.rakazo.url, &token, &space)?;
    let backend = Arc::new(RakazoBackend::new(
        client,
        store,
        config.tenant,
        config.bots,
    ));
    ipc::serve(backend, tokio::io::stdin(), tokio::io::stdout()).await
}
