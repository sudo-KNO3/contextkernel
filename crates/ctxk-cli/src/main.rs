//! `ctxk` — ContextKernel CLI.

use anyhow::{Context, Result};
use clap::{Parser as ClapParser, Subcommand};
use ctxk_retrieval::{assemble, execute, Query};
use ctxk_store::Vault;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(ClapParser, Debug)]
#[command(name = "ctxk", about = "ContextKernel — abstract knowledge plugin")]
struct Cli {
    /// Vault root directory. Defaults to $CTXK_VAULT or $HOME/.contextkernel/vault.
    #[arg(long, env = "CTXK_VAULT")]
    vault: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Initialise a fresh vault directory layout at the given path
    /// (or the default if --vault is set).
    Init {
        #[arg(default_value = "")]
        path: String,
    },

    /// Walk the vault, parse every .html file, refresh the SQLite index.
    Reindex,

    /// Start the HTTP server.
    Serve {
        #[arg(short, long, default_value = "127.0.0.1:9292")]
        bind: String,
    },

    /// Local FTS5 search across the vault — for quick poking without the server.
    Search {
        /// Free-text query (will be tokenised and OR'd as FTS5 terms).
        query: String,
        /// Scope filter (session|user|project|workspace|organization|global).
        #[arg(long)]
        scope: Option<String>,
        /// Knowledge type filter (fact|preference|constraint|...).
        #[arg(long = "type")]
        knowledge_type: Option<String>,
        /// Max items in the bundle.
        #[arg(long, default_value_t = 8)]
        max_items: usize,
    },

    /// Append a proposed knowledge item to the review queue.
    Propose {
        /// JSON file matching the propose request shape.
        #[arg(long)]
        file: PathBuf,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let vault_path = resolve_vault_path(&cli)?;

    match cli.command {
        Command::Init { ref path } => {
            let target = if path.is_empty() {
                vault_path.clone()
            } else {
                PathBuf::from(path)
            };
            init_cmd(target)
        }
        Command::Reindex => reindex_cmd(vault_path),
        Command::Serve { bind } => serve_cmd(vault_path, bind),
        Command::Search {
            query,
            scope,
            knowledge_type,
            max_items,
        } => search_cmd(vault_path, query, scope, knowledge_type, max_items),
        Command::Propose { file } => propose_cmd(vault_path, file),
    }
}

fn resolve_vault_path(cli: &Cli) -> Result<PathBuf> {
    if let Some(p) = &cli.vault {
        return Ok(p.clone());
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("could not determine home directory; set --vault or CTXK_VAULT")?;
    Ok(home.join(".contextkernel").join("vault"))
}

fn init_cmd(path: PathBuf) -> Result<()> {
    println!("Initialising ContextKernel vault at {}", path.display());
    let vault = Vault::init(&path).context("init vault")?;
    let report = vault.reindex_all().context("initial reindex")?;
    println!("  layout created");
    println!("  index at {}", vault.store.db_path().display());
    println!(
        "  initial reindex: {} files scanned, {} items, {} errors",
        report.files_scanned,
        report.items_indexed,
        report.errors.len()
    );
    println!();
    println!("Next:");
    println!("  ctxk serve                 # start HTTP server");
    println!("  ctxk search <query>        # quick FTS-only search");
    println!("  drop .html files in {}", path.display());
    println!("  ctxk reindex               # pick them up");
    Ok(())
}

fn reindex_cmd(path: PathBuf) -> Result<()> {
    let vault = Vault::open(&path).context("open vault")?;
    let report = vault.reindex_all().context("reindex")?;
    println!(
        "Reindexed {} files, {} items ({} skipped, {} errors)",
        report.files_scanned,
        report.items_indexed,
        report.files_skipped,
        report.errors.len()
    );
    for (p, e) in &report.errors {
        eprintln!("  ! {}: {}", p.display(), e);
    }
    Ok(())
}

fn serve_cmd(path: PathBuf, bind: String) -> Result<()> {
    let vault = Arc::new(Vault::open(&path).context("open vault")?);
    println!("Vault: {}", path.display());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(ctxk_server::serve(vault, &bind))
}

fn search_cmd(
    path: PathBuf,
    q: String,
    scope: Option<String>,
    knowledge_type: Option<String>,
    max_items: usize,
) -> Result<()> {
    let vault = Vault::open(&path).context("open vault")?;
    let req = Query {
        task: q.clone(),
        scope,
        knowledge_types: knowledge_type.map(|t| vec![t]),
        max_items,
        ..Default::default()
    };
    let scored = execute(&vault.store, &req).context("execute query")?;
    let bundle = assemble(&vault.store, &req, scored).context("assemble bundle")?;
    println!(
        "Query: \"{}\"  ({} candidates, {} returned, {} conflicts)",
        q,
        bundle.total_candidates,
        bundle.items.len(),
        bundle.conflicts.len()
    );
    for (i, it) in bundle.items.iter().enumerate() {
        println!(
            "  {}. [{:.3}] {} ({} · {})  ← {}",
            i + 1,
            it.score,
            it.title,
            it.knowledge_type,
            it.scope,
            it.id
        );
    }
    for c in &bundle.conflicts {
        eprintln!(
            "  ! conflict on '{}' in scope='{}': {:?}",
            c.claim_key, c.scope, c.item_ids
        );
    }
    Ok(())
}

fn propose_cmd(path: PathBuf, file: PathBuf) -> Result<()> {
    let vault = Vault::open(&path).context("open vault")?;
    let raw = std::fs::read_to_string(&file)
        .with_context(|| format!("reading {}", file.display()))?;
    let req: serde_json::Value =
        serde_json::from_str(&raw).context("propose JSON parse")?;
    let proposed_by = req
        .get("proposed_by")
        .and_then(|v| v.as_str())
        .unwrap_or("cli:user")
        .to_string();
    let rationale = req
        .get("rationale")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let item = req.get("item").cloned().unwrap_or(serde_json::json!({}));
    let payload_json = serde_json::to_string(&item)?;
    let queue_id = vault
        .store
        .queue_propose("new", None, &proposed_by, &payload_json, rationale.as_deref())
        .context("queue propose")?;
    println!("queued: {queue_id} (pending)");
    Ok(())
}
