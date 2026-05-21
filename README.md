# ContextKernel

> **Portable, structured memory for AI systems.** Local-first abstract
> knowledge plugin written in Rust. Stores knowledge as inspectable HTML
> objects with rich metadata, indexes them in SQLite + FTS5, and assembles
> precise context bundles for AI agents over a small HTTP API.

| | |
|---|---|
| Status | early MVP — session 1 (HTTP/JSON query path) shipped |
| Stack | Rust workspace (5 crates), SQLite + FTS5, axum, Python SDK |
| License | MIT |
| Binary | `ctxk` |
| Default port | `127.0.0.1:9292` |
| Default vault | `$HOME/.contextkernel/vault` (override with `--vault` or `CTXK_VAULT`) |

## Why

Most AI memory systems are opaque (chat history), unstructured (flat text),
or unreadable (raw vectors). That makes them hard to inspect, hard to edit,
and untrustworthy over time.

ContextKernel treats every memory as an HTML `<section>` with explicit
metadata — `data-knowledge-type`, `data-scope`, `data-confidence`,
`data-source-type`, `data-status`, `data-valid-until`, … — that AI agents
can filter, weight, and cite. The file on disk is the source of truth.
SQLite is a derived, rebuildable index.

## Quick start

```bash
# 1. Build
cargo build --release

# 2. Initialise a vault + seed it with the demo
./target/release/ctxk init                                # creates $HOME/.contextkernel/vault
cp -r examples/basic_vault/projects/demo \
      $HOME/.contextkernel/vault/projects/

# 3. Index the seed HTML
./target/release/ctxk reindex

# 4. Search from the CLI
./target/release/ctxk search "receptor grid units" --scope project

# 5. Start the HTTP server
./target/release/ctxk serve
```

## Querying from Python

```bash
cd py/contextkernel
pip install -e .
```

```python
from contextkernel import ContextKernel
kc = ContextKernel()  # http://127.0.0.1:9292
bundle = kc.query(
    task="receptor grid units",
    scope="project",
    knowledge_types=["constraint", "fact"],
    max_items=5,
)
for it in bundle.items:
    print(f"{it.score:.3f}  {it.title}  ({it.knowledge_type}/{it.scope})")
```

## Querying from curl

```bash
curl -X POST http://127.0.0.1:9292/context/query \
     -H "Content-Type: application/json" \
     -d '{"task":"receptor grid units","scope":"project","max_items":5}'
```

## Knowledge object format

The unit is a `<section data-knowledge-id>` with metadata attributes:

```html
<section data-knowledge-id="01HZX..."
         data-knowledge-type="constraint"
         data-scope="project"
         data-confidence="0.9"
         data-source-type="user"
         data-status="active"
         data-stability="long-term"
         data-created="2026-05-21T10:00:00Z"
         data-modified="2026-05-21T10:00:00Z"
         data-valid-until=""
         data-domain="aermod"
         data-tags="receptor-grid units">
  <h3>Receptor grid units must be meters</h3>
  <p>All AERMOD receptor grids in this project use meters, not feet.</p>
  <footer class="ctxk-meta">
    <span data-rel="supersedes" data-target="01HZX5…"></span>
  </footer>
</section>
```

Vault layout:

```
$HOME/.contextkernel/vault/
├── user/             # personal preferences, profile, writing style
├── projects/<name>/  # per-project facts, decisions, constraints, sources
├── domains/          # cross-project domain knowledge
├── templates/        # reusable report / prompt structures
├── sources/          # imported citations
└── system/index.sqlite
```

## CLI

```
ctxk init [path]              initialise a vault directory layout
ctxk reindex                  walk the vault, refresh the SQLite/FTS index
ctxk search "<query>"         local FTS+rerank search (no HTTP)
ctxk serve [--bind addr]      start the HTTP API on 127.0.0.1:9292
ctxk propose --file <json>    enqueue an AI-proposed knowledge item
```

Global: `--vault <path>` overrides the default vault location;
`CTXK_VAULT` env var does the same.

## HTTP API

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/context/query` | Assemble a context bundle for an AI task |
| `GET`  | `/knowledge/{id}` | One full item |
| `GET`  | `/knowledge?scope=&type=&domain=&tag=&q=` | List with filters |
| `POST` | `/knowledge/propose` | AI proposes new item → review queue |
| `PATCH` | `/knowledge/{id}/propose-update` | AI proposes an update |
| `GET`  | `/review/queue?status=pending` | List proposals awaiting review |
| `POST` | `/vault/reindex` | Rebuild the SQLite index from HTML |
| `GET`  | `/vault/stats` | Counts by type and scope |
| `GET`  | `/health` | `"ok"` |

`POST /context/query` body:

```json
{
  "task": "Set up receptor grid for Atlanta runs",
  "scope": "project",
  "scope_path": "demo",
  "knowledge_types": ["constraint","fact","preference"],
  "domains": ["aermod"],
  "tags_any": ["receptor-grid"],
  "include_stale": false,
  "max_items": 12,
  "include_conflicts": true
}
```

Response: scored items + per-item score breakdown + detected conflicts.

## Architecture

```
Python agents ─HTTP/JSON─► axum (127.0.0.1:9292)
                          │
                  ┌───────┴───────┐
                  ▼               ▼
            ctxk-retrieval   ctxk-review-queue
                  │
                  ▼
              ctxk-store
              │       │
              ▼       ▼
       HTML files   SQLite (FTS5 + planned sqlite-vec)
       (truth)     (index, derived)
```

Five crates:

- **`ctxk-core`** — enums, types, IDs, errors. No I/O.
- **`ctxk-store`** — HTML parse / emit, SQLite schema, FTS5, vault loader.
- **`ctxk-retrieval`** — query candidate pool, rerank, conflict detect, bundle.
- **`ctxk-server`** — axum routes (the HTTP API above).
- **`ctxk-cli`** — `ctxk` binary subcommands.

Plus a Python SDK in `py/contextkernel/`.

## What's not done yet

Tracked in `plans/i-have-a-spec-deep-pebble.md`:

- Rust-native embeddings (`fastembed-rs` + `sqlite-vec`) for true semantic search
- Approve-from-queue logic that writes HTML back to the vault
- Editor / browser UI (Tera + HTMX, no React)
- D3 force-graph view (lift from the rust-html-brain sibling project)
- MCP server adapter (`rmcp`)
- Bearer-token auth for non-loopback deployment

## License

MIT.
