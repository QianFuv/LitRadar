# Docker 部署说明

本文档说明当前 Docker 镜像与根目录 `docker-compose.yml` 的实际行为。后端镜像只包含 Rust 运行时入口，并提供 `api`、`index`、`notify`、`push`、`scheduler` 和 `worker`。

## 服务拓扑

```text
浏览器
  ├── http://localhost:3000  -> app (Next.js)
  └── http://localhost:8000  -> api (api)

worker sidecar
  └── worker
```

根 Compose 把前端和 API 分别发布到宿主机 loopback 的 `127.0.0.1:3000` 与 `127.0.0.1:8000`，并把宿主机 `./data` 挂载到 Rust 后端容器的 `/app/data`。

## Compose 服务

### `api`

- 构建上下文：仓库根目录
- Dockerfile：根目录 `Dockerfile`
- 镜像名：`ghcr.io/qianfuv/paper-scanner-api:latest`
- 启动命令：`api --host 0.0.0.0 --port 8000 --project-root /app`
- 端口：`127.0.0.1:8000:8000`
- 卷挂载：`./data:/app/data`
- 后端运行配置：来自 `data/auth.sqlite` 的管理员运行时配置

### `worker`

- 复用后端镜像
- 启动命令：`worker --project-root /app --interval-seconds 30`
- 卷挂载：`./data:/app/data`
- 依赖：`api`

`worker` 每 30 秒持久化一次检查游标和心跳，按任务的 IANA 时区与五段 cron 生成运行槽，再通过 SQLite 唯一约束和事务认领执行。多个共享 `data/auth.sqlite` 的 worker 可以安全竞争任务；同一任务同时最多运行一个实例。进程退出后，未开始的过期认领可以回收，已经开始但失去心跳的运行会标记为 `unknown`，避免自动重复产生外部副作用。需要立即执行或 dry-run 单个后台任务时，使用 `scheduler run-once TASK_ID` 或 `scheduler dry-run-once TASK_ID`。

### `app`

- 构建上下文：`./app`
- Dockerfile：`app/Dockerfile`
- 镜像名：`ghcr.io/qianfuv/paper-scanner-app:latest`
- 端口：`127.0.0.1:3000:3000`
- 环境变量：`HOSTNAME=0.0.0.0`
- 依赖：`api`

`app/Dockerfile` 的构建参数 `INTERNAL_API_URL` 默认是 `http://api:8000`，因此前端镜像会把 `/api/*` rewrite 到 Docker 网络内的 Rust API 服务。

## 后端镜像

后端镜像分两阶段构建：

1. `rust:1.96-bookworm` 构建阶段执行 release 构建
2. `debian:bookworm-slim` 运行阶段复制 `admin`、`api`、`index`、`notify`、`push`、`scheduler`、`worker`、`ps-api`、`libs/simple-linux/` 和 `data/meta/`

运行阶段默认命令为 `api --host 0.0.0.0 --port 8000 --project-root /app`。SQLite `simple` 分词扩展从镜像内 `libs/simple-linux/` 自动发现；没有单独的后端运行配置环境变量。

镜像不包含旧 Python 后端运行时。

## 快速启动

```bash
docker compose build
docker compose up -d
```

首次启动后，用安全输入或密码管理器为当前 shell 提供 `ADMIN_PASSWORD`，再通过 stdin 初始化管理员：

```bash
printf '%s\n' "$ADMIN_PASSWORD" | docker compose run --rm -T api admin bootstrap --username admin --password-stdin
```

实际密码不能写在命令参数、Compose 文件或 shell 历史中。用户表非空时该命令会失败且不会修改任何账号。

访问地址：

- 前端：`http://localhost:3000`
- API：`http://localhost:8000/api`
- HTTP MCP：`http://localhost:8000/mcp`
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

生产索引库可直接放入 `data/index/`。中文全文凭证保存在用户级 CNKI session 表；scholarly API key、CORS、MCP 和 Cookie 安全策略从 `data/auth.sqlite` 的 `runtime_settings` 读取。

## 运行时配置

后端共享配置由管理员后台写入 `runtime_settings`：

| 配置项 | 默认值 | 说明 |
| --- | --- | --- |
| `openalex_api_key_pool` | 空 | OpenAlex API key 池 |
| `semantic_scholar_api_key_pool` | 空 | Semantic Scholar API key 池 |
| `crossref_mailto_pool` | 空 | Crossref 联系邮箱池 |
| `cors_allowed_origins` | 空 | 跨源浏览器请求允许的 Origin 列表 |
| `mcp_allowed_hosts` | `localhost,127.0.0.1,::1` | HTTP MCP `Host` 白名单；非 loopback 域名或反向代理访问 `/mcp` 时必须配置 |
| `mcp_allowed_origins` | 空 | HTTP MCP 浏览器 `Origin` 白名单；仅浏览器跨源直连 MCP 时需要 |
| `secure_cookies` | `false` | `ps_session` Cookie 是否带 `Secure` 标记 |

API 在容器内仍监听 `0.0.0.0:8000` 供 Compose 网络访问，但宿主机端口默认只绑定 loopback。需要局域网或公网访问时，应通过 TLS 反向代理显式发布，并同时配置 `secure_cookies`、CORS/MCP Host 白名单和代理层共享认证限流。

## HTTP MCP 部署

Rust API 容器内置 Streamable HTTP MCP 端点 `/mcp`，不需要启动任何单独的 MCP 服务。该端点复用现有 API 认证：

- 服务器端 MCP 客户端使用 `Authorization: Bearer <access_token>`
- 同源浏览器调用可使用 `ps_session` Cookie

本地通过 `http://localhost:8000/mcp` 访问时默认可用。通过公网域名、局域网 IP 或反向代理访问时，应在管理员运行时配置中加入实际请求 Host，例如：

```yaml
mcp_allowed_hosts: paper.example,paper.example:443
```

如果 MCP 客户端运行在浏览器中且跨源直连后端，再设置 `mcp_allowed_origins`，例如 `https://app.example`。普通命令行或桌面 MCP 客户端通常只需要 Bearer Token 和 `mcp_allowed_hosts`。

## 常见问题

### 前端能打开，但搜索没有数据

1. 检查 `api` 服务：`docker compose logs api`
2. 检查宿主机 `data/index/` 下是否存在 `.sqlite` 文件
3. 如需生成索引库，运行 `docker compose run --rm api index --file <csv>` 或把现有 `.sqlite` 放入 `data/index/`

### API 请求日志太多或太少

`api` 服务默认输出 HTTP 请求日志，包含 method、path、status 和 latency。需要调整过滤级别时，在 Compose 环境中设置 `RUST_LOG`，例如 `RUST_LOG=error` 只保留 error 级日志。

### 中文搜索命中差

确认 Linux 版 `simple` 分词扩展存在于镜像内 `libs/simple-linux/`。Docker 镜像默认复制该目录。

### 通知或追踪推送没有结果

检查：

- 是否存在最新的 `data/push_state/*.changes.json`
- 用户是否在 `notification_settings` 中启用了对应投递方式
- PushPlus 或 OpenAI 兼容模型配置是否完整
- `data/auth.sqlite` 的用户通知设置和运行时配置是否完整
