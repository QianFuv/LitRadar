# Docker 部署说明

本文档说明当前 Docker 镜像与根目录 `docker-compose.yml` 的实际行为。后端镜像只包含 Rust 运行时入口，并提供旧命令名 `api`、`index`、`notify` 和 `push`。

## 服务拓扑

```text
浏览器
  ├── http://localhost:3000  -> app (Next.js)
  └── http://localhost:8000  -> api (api)

worker sidecar
  └── ps-cli worker shadow
```

根 Compose 暴露 `3000` 和 `8000`，并把宿主机 `./data` 挂载到 Rust 后端容器的 `/app/data`。

## Compose 服务

### `api`

- 构建上下文：仓库根目录
- Dockerfile：根目录 `Dockerfile`
- 镜像名：`ghcr.io/qianfuv/paper-scanner-api:latest`
- 启动命令：`api`
- 端口：`8000:8000`
- 卷挂载：`./data:/app/data`
- 关键环境变量：
  - `API_HOST=0.0.0.0`
  - `API_PORT=8000`
  - `PAPER_SCANNER_PROJECT_ROOT=/app`
  - `OPENALEX_API_KEY_POOL=${OPENALEX_API_KEY_POOL:-}`
  - `SEMANTIC_SCHOLAR_API_KEY_POOL=${SEMANTIC_SCHOLAR_API_KEY_POOL:-}`
  - `PROXY_POOL=${PROXY_POOL:-}`
  - `CROSSREF_MAILTO_POOL=${CROSSREF_MAILTO_POOL:-}`

### `worker`

- 复用后端镜像
- 启动命令：`ps-cli worker shadow --interval-seconds 300`
- 卷挂载：`./data:/app/data`
- 环境变量与 `api` 服务保持一致
- 依赖：`api`

`worker shadow` 会周期性加载并校验 `scheduled_tasks`，保持 sidecar 进程运行。需要实际执行或 dry-run 单个后台任务时，使用 `ps-cli scheduler run-once TASK_ID` 或 `ps-cli scheduler dry-run-once TASK_ID`。

### `app`

- 构建上下文：`./app`
- Dockerfile：`app/Dockerfile`
- 镜像名：`ghcr.io/qianfuv/paper-scanner-app:latest`
- 端口：`3000:3000`
- 环境变量：`HOSTNAME=0.0.0.0`
- 依赖：`api`

`app/Dockerfile` 的构建参数 `INTERNAL_API_URL` 默认是 `http://api:8000`，因此前端镜像会把 `/api/*` rewrite 到 Docker 网络内的 Rust API 服务。

## 后端镜像

后端镜像分两阶段构建：

1. `rust:1.86-bookworm` 构建阶段执行 release 构建
2. `debian:bookworm-slim` 运行阶段复制 `api`、`index`、`notify`、`push`、`ps-api`、`ps-cli`、`libs/simple-linux/` 和 `data/meta/`

运行阶段默认设置：

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `API_HOST` | `0.0.0.0` | API 监听地址 |
| `PAPER_SCANNER_PROJECT_ROOT` | `/app` | 数据目录解析根路径 |
| `SIMPLE_TOKENIZER_PATH` | `/app/libs/simple-linux/libsimple-linux-ubuntu-latest/libsimple.so` | SQLite `simple` 分词扩展 |

镜像不包含旧 Python 后端运行时。

## 快速启动

```bash
docker compose build
docker compose up -d
```

访问地址：

- 前端：`http://localhost:3000`
- API：`http://localhost:8000/api`
- API 文档：`http://localhost:8000/docs/`
- OpenAPI JSON：`http://localhost:8000/openapi.json`

健康检查：

```bash
curl http://localhost:8000/api/health
curl http://localhost:3000/api/health
```

## 数据与初始化

`api` 和 `worker` 都读取挂载的宿主机 `./data`：

- `data/meta/*.csv`：期刊元数据 CSV
- `data/index/*.sqlite`：检索数据库
- `data/auth.sqlite`：用户、收藏、通知、管理员数据
- `data/push_state/*.json`：通知、追踪和每周更新状态
- `data/push_state/*.changes.json`：增量变更清单

首次部署前应确认 `data/index/` 下已有需要服务的 `.sqlite` 索引库。需要重新生成索引时，可以在后端容器中运行 Rust CLI：

```bash
docker compose run --rm api index --file english_journals.csv --update --notify-dry-run
docker compose run --rm api index --file cnki_journals.csv --resume --issue-batch 10
```

生产索引库可直接放入 `data/index/`。中文全文凭证和 scholarly API key 等运行时配置优先从 `data/auth.sqlite` 的 `runtime_settings` 读取；没有数据库配置时才使用容器环境变量。

## 常用环境变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `API_HOST` | `0.0.0.0` | API 监听地址 |
| `API_PORT` | `8000` | API 监听端口 |
| `API_CORS_ALLOWED_ORIGINS` | 空 | 跨源浏览器请求允许的 Origin 列表 |
| `AUTH_COOKIE_SECURE` | 按请求 scheme 推断 | 显式控制 `ps_session` Cookie 的 `Secure` 标记 |
| `OPENALEX_API_KEY_POOL` | 空 | OpenAlex API key 池 |
| `SEMANTIC_SCHOLAR_API_KEY_POOL` | 空 | Semantic Scholar API key 池 |
| `CROSSREF_MAILTO_POOL` | 空 | Crossref 联系邮箱池 |
| `PROXY_POOL` | 空 | scholarly 与 CNKI 请求代理池 |
| `NOTIFY_AI_BASE_URL` | `https://api.siliconflow.cn/v1` | 默认 OpenAI 兼容 API 地址 |
| `NOTIFY_AI_API_KEY` | 空 | 默认 AI key |
| `NOTIFY_AI_MODEL` | `deepseek-ai/DeepSeek-V3` | 默认模型名 |

管理员后台写入 `runtime_settings` 后，Rust API、Rust worker 和 Rust CLI 会优先使用数据库中的值。

## 常见问题

### 前端能打开，但搜索没有数据

1. 检查 `api` 服务：`docker compose logs api`
2. 检查宿主机 `data/index/` 下是否存在 `.sqlite` 文件
3. 如需生成索引库，运行 `docker compose run --rm api index --file <csv>` 或把现有 `.sqlite` 放入 `data/index/`

### API 请求日志太多或太少

`api` 服务默认输出 HTTP 请求日志，包含 method、path、status 和 latency。需要调整过滤级别时，在 Compose 环境中设置 `RUST_LOG`，例如 `RUST_LOG=error` 只保留 error 级日志。

### 中文搜索命中差

确认 `SIMPLE_TOKENIZER_PATH` 指向的 Linux 版 `simple` 分词扩展存在。Docker 镜像默认复制 `libs/simple-linux/`。

### 通知或追踪推送没有结果

检查：

- 是否存在最新的 `data/push_state/*.changes.json`
- 用户是否在 `notification_settings` 中启用了对应投递方式
- PushPlus 或 OpenAI 兼容模型配置是否完整
- `data/auth.sqlite` 的 `runtime_settings` 是否覆盖了预期环境变量
