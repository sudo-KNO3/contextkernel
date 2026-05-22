//! `ctxk` — ContextKernel CLI.

use anyhow::{Context, Result};
use clap::{Parser as ClapParser, Subcommand};
use ctxk_core::EmbedderProvider;
use ctxk_embed::Embedder;
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

    /// Disable semantic search (no embedding model loaded).
    #[arg(long, global = true)]
    no_embed: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Initialise a fresh vault directory layout.
    Init {
        #[arg(default_value = "")]
        path: String,
    },

    /// Walk the vault, parse every .html file, refresh the SQLite index.
    /// Computes embeddings for new/changed items unless --no-embed.
    Reindex,

    /// Force a full re-embed of every item (e.g. after model change).
    Reembed,

    /// Start the HTTP server.
    Serve {
        #[arg(short, long, default_value = "127.0.0.1:9292")]
        bind: String,
    },

    /// Local search across the vault — useful without the server.
    Search {
        query: String,
        #[arg(long)]
        scope: Option<String>,
        #[arg(long = "type")]
        knowledge_type: Option<String>,
        #[arg(long, default_value_t = 8)]
        max_items: usize,
    },

    /// Append a proposed knowledge item to the review queue.
    Propose {
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

    // Build the embedder once unless the user opted out. fastembed is lazy —
    // the actual model load happens on first embed call, so this is free.
    let embedder: Option<Arc<dyn EmbedderProvider>> = if cli.no_embed {
        None
    } else {
        Some(Arc::new(Embedder))
    };

    match cli.command {
        Command::Init { ref path } => {
            let target = if path.is_empty() { vault_path.clone() } else { PathBuf::from(path) };
            init_cmd(target, embedder.as_deref())
        }
        Command::Reindex => reindex_cmd(vault_path, embedder.as_deref()),
        Command::Reembed => reembed_cmd(vault_path, embedder.as_deref()),
        Command::Serve { bind } => serve_cmd(vault_path, bind, embedder),
        Command::Search { query, scope, knowledge_type, max_items } => {
            search_cmd(vault_path, query, scope, knowledge_type, max_items, embedder.as_deref())
        }
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

fn init_cmd(path: PathBuf, embedder: Option<&dyn EmbedderProvider>) -> Result<()> {
    println!("Initialising ContextKernel vault at {}", path.display());
    let vault = Vault::init(&path).context("init vault")?;
    let report = vault.reindex_all(embedder).context("initial reindex")?;
    println!("  layout created");
    println!("  index at {}", vault.store.db_path().display());
    println!(
        "  initial reindex: {} files, {} items, {} embeddings, {} errors",
        report.files_scanned,
        report.items_indexed,
        report.items_embedded,
        report.errors.len()
    );
    println!();
    println!("Next:");
    println!("  ctxk serve                 # start HTTP server");
    println!("  ctxk search <query>        # local search");
    println!("  drop .html files in {}", path.display());
    println!("  ctxk reindex               # pick them up");
    Ok(())
}

fn reindex_cmd(path: PathBuf, embedder: Option<&dyn EmbedderProvider>) -> Result<()> {
    let vault = Vault::open(&path).context("open vault")?;
    let report = vault.reindex_all(embedder).context("reindex")?;
    println!(
        "Reindexed {} files, {} items, {} embeddings ({} skipped, {} errors)",
        report.files_scanned,
        report.items_indexed,
        report.items_embedded,
        report.files_skipped,
        report.errors.len()
    );
    for (p, e) in &report.errors {
        eprintln!("  ! {}: {}", p.display(), e);
    }
    Ok(())
}

fn reembed_cmd(path: PathBuf, embedder: Option<&dyn EmbedderProvider>) -> Result<()> {
    if embedder.is_none() {
        anyhow::bail!("--no-embed and `reembed` are mutually exclusive");
    }
    let vault = Vault::open(&path).context("open vault")?;
    // Force re-embed by clearing the recorded model — reindex_all detects mismatch.
    vault.store.set_meta("embedding_model", "")?;
    let report = vault.reindex_all(embedder).context("reembed")?;
    println!(
        "Re-embedded {} items across {} files",
        report.items_embedded, report.files_scanned
    );
    Ok(())
}

fn serve_cmd(
    path: PathBuf,
    bind: String,
    embedder: Option<Arc<dyn EmbedderProvider>>,
) -> Result<()> {
    let vault = Arc::new(Vault::open(&path).context("open vault")?);
    println!("Vault: {}", path.display());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(ctxk_server::serve(vault, embedder, &bind))
}

fn search_cmd(
    path: PathBuf,
    q: String,
    scope: Option<String>,
    knowledge_type: Option<String>,
    max_items: usize,
    embedder: Option<&dyn EmbedderProvider>,
) -> Result<()> {
    let vault = Vault::open(&path).context("open vault")?;
    let req = Query {
        task: q.clone(),
        scope,
        knowledge_types: knowledge_type.map(|t| vec![t]),
        max_items,
        ..Default::default()
    };
    let scored = execute(&vault.store, &req, embedder).context("execute query")?;
    let bundle = assemble(&vault.store, &req, scored).context("assemble bundle")?;
    let mode = if embedder.is_some() { "semantic+lexical" } else { "lexical-only" };
    println!(
        "Query: \"{}\"  [{}]  ({} candidates, {} returned, {} conflicts)",
        q,
        mode,
        bundle.total_candidates,
        bundle.items.len(),
        bundle.conflicts.len()
    );
    for (i, it) in bundle.items.iter().enumerate() {
        println!(
            "  {}. [{:.3} | sem {:.3} fts {:.0} lex {:.3}] {} ({} · {})",
            i + 1,
            it.score,
            it.score_breakdown.semantic,
            it.score_breakdown.fts,
            it.score_breakdown.lexical,
            it.title,
            it.knowledge_type,
            it.scope
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
