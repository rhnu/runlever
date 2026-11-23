mod auth;
mod http;
mod service;

use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

use runlever::{config::Catalog, engine::Engine};

const DEFAULT_CONFIG: &str = "/etc/runlever/config.toml";

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate action definitions without running commands.
    Check {
        #[arg(long, default_value = DEFAULT_CONFIG)]
        config: PathBuf,
    },
    /// Start the HTTP server and local executor.
    Serve {
        #[arg(long, default_value = DEFAULT_CONFIG)]
        config: PathBuf,
        #[arg(long, default_value = "/var/lib/runlever")]
        state_dir: PathBuf,
    },
    /// Manage the Linux system service.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
}

#[derive(Subcommand)]
enum ServiceCommand {
    /// Install or upgrade Runlever and enable its systemd service.
    Install {
        /// External HTTPS origin. Required for login at a public hostname.
        #[arg(long, default_value = "https://localhost")]
        origin: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "runlever=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Command::Check { config } => {
            let catalog = Catalog::load(&config)?;
            println!("配置有效（{} 个动作）", catalog.actions().len());
            Ok(())
        }
        Command::Service {
            command: ServiceCommand::Install { origin },
        } => service::install(&origin),
        Command::Serve { config, state_dir } => {
            let catalog = Arc::new(Catalog::load(&config)?);
            catalog.check_secrets()?;
            fs::create_dir_all(&state_dir).context("create state directory")?;
            let guard = File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(state_dir.join("instance.lock"))
                .context("open instance lock")?;
            guard
                .try_lock()
                .context("another Runlever process may be using this state directory")?;
            let listener = tokio::net::TcpListener::bind(catalog.listen())
                .await
                .context("bind HTTP listener")?;
            let engine = Engine::open(&state_dir, catalog.retention_days(), catalog.secrets())?;
            info!(address = %listener.local_addr()?, "serving Runlever");
            axum::serve(listener, http::router(catalog, engine))
                .await
                .context("serve HTTP")
        }
    }
}
