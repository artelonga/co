//! co-lsp — Language Server Protocol server for CO universes.
//!
//! Usage:
//!   co-lsp [--url <url>] [--token <token>] [--universe <slug>]
//!
//! Falls back to ~/.config/co/credentials if --token is omitted.
//! Communicates over stdin/stdout (standard LSP transport).

mod backend;
mod config;
mod vault_client;

use clap::Parser;
use tower_lsp::{LspService, Server};

use backend::CoLspBackend;
use config::LspConfig;

#[derive(Parser)]
#[command(name = "co-lsp", about = "CO Language Server")]
struct Args {
    /// CO server base URL (default: https://co.artelonga.com.br or from credentials)
    #[arg(long)]
    url: Option<String>,

    /// CO API token (co_… — falls back to ~/.config/co/credentials)
    #[arg(long)]
    token: Option<String>,

    /// Universe slug to pre-load entry index for
    #[arg(long)]
    universe: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let config = LspConfig::resolve(args.url, args.token, args.universe)?;

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| CoLspBackend::new(client, config));
    Server::new(stdin, stdout, socket).serve(service).await;

    Ok(())
}
