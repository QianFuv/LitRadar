# Paper Scanner

A full-stack application for aggregating, indexing, and searching academic journal articles. Paper Scanner fetches article metadata from BrowZine API and WeipuAPI, stores it in SQLite databases with full-text search, and provides a modern web interface for browsing and discovery. It also supports AI-powered notification workflows to alert users about newly published articles matching their research interests.

## Features

- **Multi-source indexing** - Fetches journal and article metadata from BrowZine API and WeipuAPI (Chinese journals)
- **Full-text search** - SQLite FTS5-based search across article titles and abstracts, with CJK tokenizer support
- **Advanced filtering** - Filter by journal, research area, publication year, date range, open access status, and more
- **Weekly updates** - Track and browse newly added articles per journal
- **AI-powered notifications** - Uses SiliconFlow LLM to select relevant articles and push notifications via PushPlus
- **Incremental sync** - Change tracking with article-level diff for efficient updates
- **Multi-process indexing** - Parallel journal processing for large-scale data collection

## Screenshots

![](https://i.see.you/2026/02/10/W8if/d9c1cb7.png)

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Frontend | Next.js 16, React 19, TypeScript, TailwindCSS 4, Radix UI |
| Backend API | FastAPI, Uvicorn, aiosqlite |
| Database | SQLite (WAL mode, FTS5) |
| Data Fetching | httpx, Playwright, selectolax |
| AI/Notification | OpenAI SDK (SiliconFlow), PushPlus |
| Deployment | Docker, GitHub Actions, GHCR |
| Dev Tools | uv, Ruff, mypy |

## Quick Start

### Prerequisites

- Docker and Docker Compose

### 1. Download and Configure

Download `docker-compose.yml`, then create the directory structure:

```
project/
├── docker-compose.yml
├── config/
│   └── auth.yaml           # Frontend authentication tokens
└── data/
    └── meta/               # Journal metadata CSV files
```

### 2. Prepare Data

Place journal metadata CSV files in `data/meta/`. Each CSV must have the following columns:

| Column | Description |
|--------|-------------|
| `title` | Journal name |
| `issn` | Journal ISSN |
| `id` | BrowZine journal ID |
| `area` | Research area (e.g., Accounting, Finance) |
| `library` | BrowZine library ID (optional, defaults to 3050) |

Example (`data/meta/utd24.csv`):

```csv
title,issn,id,area,library
The Accounting Review,0001-4826,34781,Accounting,3050
Journal of Accounting and Economics,0165-4101,4204,Accounting,3050
```

### 3. Pull Images and Build Index

```bash
docker compose pull
docker compose run --rm api uv run index
```

This fetches article data from BrowZine/WeipuAPI and creates SQLite databases in `data/index/`.

### 4. Start the Application

```bash
docker compose up -d
```

Visit http://localhost:3000. Only port 3000 is exposed — the frontend proxies API requests to the backend internally.

### Updating

```bash
# Incremental index update
docker compose run --rm api uv run index --update

# Send notifications
docker compose run --rm api uv run notify
```

See [Docker Deployment docs](docs/docker.md) for full details on architecture, CI/CD, environment variables, and troubleshooting.

## Project Structure

```
Paper-Scanner/
├── app/                        # Next.js frontend
│   ├── app/                    # App router pages
│   │   ├── login/              # Login page
│   │   └── (protected)/        # Auth-protected routes
│   │       ├── page.tsx        # Main search page
│   │       ├── articles/       # Article detail page
│   │       └── weekly-updates/ # Weekly updates page
│   ├── components/
│   │   ├── feature/            # Business components
│   │   └── ui/                 # Radix UI primitives
│   └── lib/                    # Utilities
├── scripts/                    # Python backend
│   ├── api/                    # FastAPI REST API
│   │   ├── routes/             # Endpoint handlers
│   │   ├── queries/            # SQL query builders
│   │   ├── models.py           # Pydantic response models
│   │   ├── pagination.py       # Cursor/offset pagination
│   │   └── dependencies.py     # DI (database connections)
│   ├── index/                  # Data indexer
│   │   ├── db/                 # Schema, client, operations
│   │   ├── fetcher.py          # Journal/issue/article fetcher
│   │   ├── changes.py          # Change detection & manifests
│   │   └── workers.py          # Multi-process workers
│   ├── notify/                 # Notification system
│   │   ├── workflow.py         # Notification pipeline
│   │   └── models.py           # Notification config models
│   ├── browzine/               # BrowZine API client
│   ├── weipu/                  # WeipuAPI client
│   └── shared/                 # Shared constants & utilities
├── data/
│   ├── meta/                   # Journal metadata CSV files
│   ├── index/                  # SQLite databases (generated)
│   └── push_state/             # Notification delivery state
└── pyproject.toml
```

## CLI Commands

All CLI commands run inside the API container via `docker compose run --rm api`.

### `index` - Build/update article index

```bash
# Index all CSV files in data/meta/
docker compose run --rm api uv run index

# Index a specific CSV
docker compose run --rm api uv run index --file utd24.csv

# Incremental update with change tracking
docker compose run --rm api uv run index --update

# Update and send notifications
docker compose run --rm api uv run index --update --notify

# Parallel processing
docker compose run --rm api uv run index --workers 16 --processes 4

# Dry-run notifications (no actual delivery)
docker compose run --rm api uv run index --update --notify --notify-dry-run
```

| Flag | Default | Description |
|------|---------|-------------|
| `--file, -f` | all CSVs | Specific CSV filename under `data/meta/` |
| `--workers, -w` | 8 | Max concurrent HTTP requests |
| `--processes` | 1 | Process workers for journal-level parallelism |
| `--issue-batch` | workers*3 | Issues per async batch |
| `--timeout` | 20 | HTTP timeout in seconds |
| `--resume / --no-resume` | enabled | Resume from completed journals/years |
| `--update / --no-update` | disabled | Incremental update mode with change tracking |
| `--notify / --no-notify` | disabled | Send notifications after update (requires `--update`) |
| `--notify-dry-run` | disabled | Preview notification selection without sending |

### `api` - REST API server

Started automatically by `docker compose up`. Listens on port 8000 (internal only, proxied by frontend).

### `notify` - Run notification pipeline

```bash
# Run with default settings
docker compose run --rm api uv run notify

# Specify database and change manifest
docker compose run --rm api uv run notify --db utd24.sqlite --changes-file data/push_state/utd24.changes.json

# Dry run
docker compose run --rm api uv run notify --dry-run
```

| Flag | Default | Description |
|------|---------|-------------|
| `--db` | auto-detect | Database file under `data/index/` |
| `--changes-file` | - | Change manifest from index update |
| `--siliconflow-model` | config default | Override LLM model ID |
| `--max-candidates` | config default | Max articles sent to LLM per run |
| `--timeout` | 60 | HTTP timeout in seconds |
| `--retries` | 3 | Retry count for API calls |
| `--dedupe-retention-days` | 60 | Days to keep delivery records |
| `--dry-run` | disabled | Run selection without sending |

## API Reference

All endpoints are prefixed with `/api`. Pass `?db=<name>` to select a specific database.

### Articles

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/articles` | List articles with filtering, search, and pagination |
| `GET` | `/api/articles/{id}` | Get a single article by ID |
| `GET` | `/api/articles/{id}/fulltext` | Redirect to DOI or full-text file |

**Query parameters for `/api/articles`:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `db` | string | Database name |
| `q` | string | Full-text search query |
| `journal_id` | int | Filter by journal |
| `issue_id` | int | Filter by issue |
| `area` | string | Filter by research area |
| `year` | int | Filter by publication year |
| `date_from` | string | Start date (YYYY-MM-DD) |
| `date_to` | string | End date (YYYY-MM-DD) |
| `open_access` | int | Filter by open access status |
| `in_press` | int | Filter by in-press status |
| `limit` | int | Page size (max 200) |
| `offset` | int | Offset for pagination |
| `cursor` | string | Cursor for keyset pagination |
| `sort` | string | Sort field (e.g., `-date`, `date:desc`) |

### Journals

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/journals` | List journals with filtering and sorting |
| `GET` | `/api/journals/{id}` | Get a single journal by ID |

### Issues

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/issues` | List issues with filtering |
| `GET` | `/api/issues/{id}` | Get a single issue by ID |

### Metadata

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/meta/databases` | List available databases |
| `GET` | `/api/meta/areas` | List research areas with article counts |
| `GET` | `/api/meta/journals` | List journal options (ID + title) |
| `GET` | `/api/meta/libraries` | List libraries with journal counts |
| `GET` | `/api/years` | List years with issue/journal counts |

### Weekly Updates

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/weekly-updates` | Get recent article updates grouped by journal |

### Health

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/health` | Health check (returns `{"status": "ok"}`) |

## Notification System

Paper Scanner reads notification subscribers from `auth.sqlite` and user tracking settings in the app.
Legacy `data/push/subscriptions*.json` files are ignored.

### Setup

1. Configure per-user delivery in the tracking UI.

2. Set optional runtime defaults:

```bash
export NOTIFY_SILICONFLOW_API_KEY="sk-your-siliconflow-key"
export NOTIFY_SILICONFLOW_MODEL="deepseek-ai/DeepSeek-V3"
export NOTIFY_MAX_CANDIDATES="120"
export NOTIFY_TEMPERATURE="0.2"
export NOTIFY_PUSHPLUS_CHANNEL="mail"
export NOTIFY_PUSHPLUS_TEMPLATE="markdown"
```

3. Run with the index update pipeline:

```bash
uv run index --update --notify
```

### How It Works

1. The indexer detects changed articles by comparing before/after snapshots
2. A change manifest records added and removed article IDs
3. The notification pipeline loads candidate articles from the manifest
4. SiliconFlow LLM optionally scores articles against each user's keywords and research directions
5. Selected articles are delivered via PushPlus or the user's tracking folder
6. Delivery state is persisted to prevent duplicate notifications (60-day window)

## Database Schema

Each CSV file produces a corresponding SQLite database in `data/index/`. The schema includes:

| Table | Description |
|-------|-------------|
| `journals` | Journal metadata (ID, title, ISSN, SJR rank, etc.) |
| `journal_meta` | CSV-sourced metadata (area, source file) |
| `issues` | Journal issues (volume, number, date, year) |
| `articles` | Full article records (title, authors, abstract, DOI, URLs) |
| `article_listing` | Materialized view for optimized list queries |
| `article_search` | FTS5 virtual table for full-text search |
| `journal_year_state` | Indexing progress tracker per journal/year |
| `journal_state` | Indexing progress tracker per journal |
| `listing_state` | Whether `article_listing` is ready for queries |

## Development

### Local Setup

Requires Python 3.12+, [uv](https://docs.astral.sh/uv/), and Node.js 20+.

```bash
# Install Python dependencies
uv sync
uv sync --extra dev

# Install frontend dependencies
cd app
pnpm install
cd ..

# Start backend API (port 8000)
uv run api

# Start frontend dev server (port 3000) in another terminal
cd app
pnpm run dev
```

### Code Quality

```bash
# Lint
uv run ruff check

# Format
uv run ruff format

# Type check
uv run mypy
```

### Frontend Development

```bash
cd app
pnpm run dev      # Dev server with hot reload
pnpm run build    # Production build
pnpm run lint     # ESLint check
```

## License

This project is licensed under the MIT License. See `LICENSE`.

The bundled SQLite simple tokenizer binaries are from the upstream
`wangfenjin/simple` project. See `libs/simple/README.md` for upstream details.
