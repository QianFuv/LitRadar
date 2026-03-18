# Docker Deployment

## Architecture

```
                    ┌─────────────────────┐
   :3000            │   app (Next.js)     │
 ◄──────────────────│   node:20-alpine    │
   exposed          │                     │
                    └────────┬────────────┘
                             │ rewrites /api/*
                             │ (internal network)
                    ┌────────▼────────────┐
                    │   api (FastAPI)     │
                    │   python:3.12       │
                    │   slim-trixie       │
                    └─────────────────────┘
                        :8000 (internal only)
```

Only port 3000 is exposed. Frontend proxies API requests to the backend via Next.js rewrites over the Docker internal network.

## Quick Start

### Local Build

```bash
docker compose build
docker compose up -d
```

### Pull from GHCR

```bash
docker compose pull
docker compose up -d
```

Visit http://localhost:3000.

## Directory Layout (Deployment Server)

```
project/
├── docker-compose.yml
├── data/
│   ├── index/          # SQLite databases (mounted into api container)
│   ├── meta/           # Journal metadata CSV (baked into image, volume overrides)
│   └── push_state/     # Notification delivery state
└── config/
    └── auth.yaml       # Frontend authentication tokens
```

## Configuration

### Frontend Auth (`config/auth.yaml`)

```yaml
tokens:
  - "your-access-token"
secret: "your-signing-secret"
ttl_hours: 168
```

Mounted read-only into the app container. Changes take effect after `docker compose restart app`.

### Notification

Notification subscribers are stored in `auth.sqlite` and managed from the tracking UI.
Optional AI and PushPlus defaults come from environment variables on the `api` service.
Legacy `data/push/subscriptions*.json` files are ignored.

### Environment Variables

| Variable | Service | Default | Description |
|----------|---------|---------|-------------|
| `API_HOST` | api | `0.0.0.0` | Uvicorn bind address |
| `NOTIFY_SILICONFLOW_API_KEY` | api | empty | Enables AI filtering for notifications |
| `NOTIFY_SILICONFLOW_MODEL` | api | `deepseek-ai/DeepSeek-V3` | Default SiliconFlow model |
| `NOTIFY_MAX_CANDIDATES` | api | `120` | Max candidates sent to the model |
| `NOTIFY_TEMPERATURE` | api | `0.2` | SiliconFlow temperature |
| `NOTIFY_PUSHPLUS_CHANNEL` | api | `mail` | Default PushPlus channel |
| `NOTIFY_PUSHPLUS_TEMPLATE` | api | `markdown` | Default PushPlus template |
| `NOTIFY_PUSHPLUS_TOPIC` | api | empty | Default PushPlus topic |
| `NOTIFY_PUSHPLUS_OPTION` | api | empty | Default PushPlus option |
| `SIMPLE_TOKENIZER_PATH` | api | Set in Dockerfile | Path to `libsimple.so` for CJK FTS |
| `HOSTNAME` | app | `0.0.0.0` | Next.js listen address |
| `INTERNAL_API_URL` | app (build) | `http://api:8000` | Backend URL for Next.js rewrites |

## Running CLI Commands

The API image includes all backend code. Use `docker compose run` for one-off tasks:

```bash
# Update index
docker compose run --rm api uv run index --update

# Send notifications
docker compose run --rm api uv run notify
```

## Image Details

### Backend (`Dockerfile`)

- **Base**: `python:3.12-slim-trixie` (glibc 2.38+ required by `libsimple.so`)
- **Build**: Multi-stage — uv installs dependencies, then copies venv to runtime stage
- **Includes**: `libs/simple-linux/` (SQLite tokenizer), `data/meta/` (journal CSV)
- **Data**: SQLite databases mounted at runtime via `./data:/app/data`

### Frontend (`app/Dockerfile`)

- **Base**: `node:20-alpine`
- **Build**: Multi-stage — pnpm install, Next.js standalone build
- **Rewrites**: `/api/*` proxied to `http://api:8000` (baked in at build time via `INTERNAL_API_URL`)
- **Config**: `auth.yaml` mounted at runtime via `./config:/app/config`

## CI/CD

GitHub Actions (`.github/workflows/docker.yml`) builds and pushes both images to GHCR on every push to `main`:

- `ghcr.io/qianfuv/paper-scanner-api:latest`
- `ghcr.io/qianfuv/paper-scanner-app:latest`

Each build also gets a `sha-<commit>` tag for rollback.

### Deploying Updates

```bash
docker compose pull
docker compose up -d
```

## Troubleshooting

### `no such tokenizer: simple`

The `libsimple.so` extension failed to load. Check:

1. **glibc version**: The image must use glibc 2.38+ (Debian Trixie or Ubuntu 24.04). Verify: `docker compose exec api ldd /app/libs/simple-linux/libsimple-linux-ubuntu-latest/libsimple.so`
2. **Environment variable**: `SIMPLE_TOKENIZER_PATH` must be set. Verify: `docker compose exec api env | grep SIMPLE`
3. **Stale image**: Run `docker compose build api` to ensure you're using the latest Dockerfile

### Frontend returns 502 or API errors

Check if the backend is running: `docker compose logs api`. Ensure `data/index/` contains at least one `.sqlite` file.

### Config changes not taking effect

- `auth.yaml`: `docker compose restart app`
- `NOTIFY_*`: `docker compose restart api`
- `INTERNAL_API_URL`: Requires `docker compose build app` (baked in at build time)
