#![forbid(unsafe_code)]
#![deny(unused_must_use)]
#![cfg_attr(
    not(test),
    deny(
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::unwrap_used
    )
)]

mod agents;
mod cli;
mod fetch;
mod fs;
mod memory;
mod policy;
mod shell;
mod skills;
mod support;

use clap::Parser;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "tuls=warn".into()))
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(false),
        )
        .init();

    match cli::Cli::parse().command {
        cli::Command::Filesystem(options) => fs::run(options).await,
        cli::Command::Fetch(options) => fetch::run(options).await,
        cli::Command::Memory(options) => memory::run(options).await,
        cli::Command::Shell(options) => shell::run(options).await,
        cli::Command::Skills(options) => skills::run(options).await,
        cli::Command::Agents(options) => agents::run(options).await,
    }
}
