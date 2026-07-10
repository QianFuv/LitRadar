# Docker 部署说明

本文档说明当前 Docker 镜像与根目录 `docker-compose.yml` 的实际行为。后端镜像只包含 Rust 运行时入口，并提供 `admin`、`api`、`index`、`notify`、`push`、`scheduler` 和 `worker`。

## 服务拓扑

```text
浏览器
  ├── http://localhost:3000  -> app (Next.js)
  └── http://localhost:8000  -> api (api)

worker sidecar
  └── worker
```

根 Compose 把前端和 API 分别发布到宿主机 loopback 的 `127.0.0.1:3000` 与 `127.0.0.1:8000`，把宿主机 `./data` 挂载到 Rust 后端容器的 `/app/data`，并把 `./secrets/paper-scanner.key` 作为只读 Compose secret 挂载到 `/run/secrets/paper_scanner_key`。密钥不进入镜像或数据卷。

## Compose 服务

### `api`

- 构建上下文：仓库根目录
- Dockerfile：根目录 `Dockerfile`
- 镜像名：`ghcr.io/qianfuv/paper-scanner-api:latest`
- 启动命令：`api --host 0.0.0.0 --port 8000 --project-root /app --secret-key-file /run/secrets/paper_scanner_key`
- 端口：`127.0.0.1:8000:8000`
- 卷挂载：`./data:/app/data:rw`
- 后端运行配置：来自 `data/auth.sqlite` 的管理员运行时配置
- 容器用户：`paper`（UID/GID `10001:10001`）
- 健康检查：容器内请求 `/api/health`

### `worker`

- 复用后端镜像
- 启动命令：`worker --project-root /app --secret-key-file /run/secrets/paper_scanner_key --interval-seconds 30`
- 卷挂载：`./data:/app/data:rw`
- 依赖：`api`
- 容器用户：`paper`（UID/GID `10001:10001`）
- 健康检查：通过 API 请求 `/api/health/worker`，读取最近 90 秒的持久心跳

`worker` 每 30 秒持久化一次检查游标和心跳，按任务的 IANA 时区与五段 cron 生成运行槽，再通过 SQLite 唯一约束和事务认领执行。多个共享 `data/auth.sqlite` 的 worker 可以安全竞争任务；同一任务同时最多运行一个实例。进程退出后，未开始的过期认领可以回收，已经开始但失去心跳的运行会标记为 `unknown`，避免自动重复产生外部副作用。需要立即执行或 dry-run 单个后台任务时，使用 `scheduler run-once TASK_ID` 或 `scheduler dry-run-once TASK_ID`。

### `app`

- 构建上下文：`./app`
- Dockerfile：`app/Dockerfile`
- 镜像名：`ghcr.io/qianfuv/paper-scanner-app:latest`
- 端口：`127.0.0.1:3000:3000`
- 环境变量：`HOSTNAME=0.0.0.0`
- 依赖：`api`
- 容器用户：Node 镜像内置的 `node` 非 root 用户
- 健康检查：容器内请求 Next.js 的 `/api/health`，同时验证内部 API rewrite

`app/Dockerfile` 的构建参数 `INTERNAL_API_URL` 默认是 `http://api:8000`，因此前端镜像会把 `/api/*` rewrite 到 Docker 网络内的 Rust API 服务。

## 后端镜像

后端镜像分两阶段构建：

1. `rust:1.96-bookworm` 构建阶段执行 release 构建
2. `debian:bookworm-slim` 运行阶段复制 `admin`、`api`、`index`、`notify`、`push`、`scheduler`、`worker`、`ps-api`、`libs/simple-linux/` 和 `data/meta/`

运行阶段默认命令仍只提供通用 API 参数；根 Compose 用包含显式密钥文件参数的命令覆盖它。直接 `docker run` 时必须自行只读挂载密钥并传入 `--secret-key-file`。SQLite `simple` 分词扩展从镜像内 `libs/simple-linux/` 自动发现；没有后端凭据环境变量。运行镜像安装 CA 证书和 `curl` 供 HTTPS 工作流与容器健康探针使用，然后切换到固定的非 root `paper` 用户。

镜像不包含旧 Python 后端运行时。

## 快速启动

```bash
sudo chown -R 10001:10001 data  # 仅 Linux 原生 Docker Engine
mkdir -p secrets
openssl rand -out secrets/paper-scanner.key 32
chmod 600 secrets/paper-scanner.key
docker compose build
docker compose up -d
```

已有明文凭据的部署不能直接启动：先保持服务停止，备份 `data/auth.sqlite`，然后按 [安全说明](security.md) 执行 `admin secrets migrate` 与 `admin secrets verify`。密钥文件必须恰好 32 字节；丢失后无法恢复数据库中的集成凭据。

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
curl --fail http://localhost:8000/api/health/worker
curl http://localhost:3000/api/health
docker compose ps
```

worker 刚启动但还未写入首个心跳时，`/api/health/worker` 返回 `503`；首轮调度后转为 `200`。worker 停止心跳超过 90 秒后重新转为 `503`。该接口不返回 worker 或任务细节。

备份不要写入 `/app/data`，也不要把 `secrets/` 与数据放进同一归档。运行镜像已经包含 `admin backup create|verify|restore`；通过额外的 `/backups` bind mount 使用它，并在离线恢复前停止三个服务、等待心跳过期。命令、确认开关、可选索引/推送状态语义和回滚流程见 [备份与离线恢复](backup.md)。

## 数据与初始化

`api` 和 `worker` 都读取挂载的宿主机 `./data`：

- `data/meta/*.csv`：期刊元数据 CSV
- `data/index/*.sqlite`：检索数据库
- `data/auth.sqlite`：用户、收藏、通知、管理员数据
- `data/push_state/*.json`：通知、追踪和每周更新状态
- `data/push_state/*.changes.json`：增量变更清单

后端镜像固定使用 UID/GID `10001:10001`。Linux 原生 Docker Engine 不转换 bind mount 所有权，因此启动前必须让该账号可读写整个 `data` 目录；可使用上面的 `chown`，或由运维系统配置等效 ACL。不要把容器改回 root。Docker Desktop for macOS/Windows 通常由虚拟化层处理挂载权限。

首次部署前应确认 `data/index/` 下已有需要服务的 `.sqlite` 索引库。需要重新生成索引时，可以在后端容器中运行 Rust CLI：

```bash
docker compose run --rm api index --secret-key-file /run/secrets/paper_scanner_key --file english_journals.csv --update --notify-dry-run
docker compose run --rm api index --secret-key-file /run/secrets/paper_scanner_key --file cnki_journals.csv --resume --issue-batch 10
```

生产索引库可直接放入 `data/index/`。中文全文 session、用户级 PushPlus/AI key 和 scholarly API key 池以版本化密文保存在 `data/auth.sqlite`；CORS、MCP 和 Cookie 安全策略仍以普通运行配置保存。设置 API 只返回秘密项的配置状态与固定掩码。

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

## 运行时安全与生产覆盖

根 Compose 为全部服务设置：

- `restart: unless-stopped`
- `read_only: true`
- `/tmp` 的 `noexec,nosuid` tmpfs
- 前端图片优化缓存使用仅对 `node` 用户可写的 `/app/.next/cache` tmpfs
- `cap_drop: [ALL]`
- `security_opt: [no-new-privileges:true]`

后端只通过 `./data:/app/data:rw` 保留业务写入。API 和 app 必须先通过健康检查，依赖服务才启动。Docker 的 unhealthy 状态不会自行重启仍在运行的进程；`restart: unless-stopped` 负责非零退出和 daemon 重启后的恢复。

生产环境应先通过本地受控启动或管理员后台把 `secure_cookies` 设置为 `true`，停止服务，然后保存以下覆盖为 `compose.production.yaml`。示例中的 `!reset` 需要 Docker Compose 2.24.4 或更高版本：

```yaml
services:
  api:
    ports: !reset []
    command:
      - api
      - --host
      - 0.0.0.0
      - --port
      - "8000"
      - --project-root
      - /app
      - --secret-key-file
      - /run/secrets/paper_scanner_key
      - --require-secure-cookies
  app:
    ports: !reset []
```

使用覆盖启动：

```bash
docker compose -f docker-compose.yml -f compose.production.yaml up -d
```

`!reset []` 移除 API 和 app 的宿主机端口，`--require-secure-cookies` 会在 API 绑定端口前验证数据库设置；仍为 `false` 时启动失败。生产编排必须再加入同一 Compose 网络中的 TLS 反向代理服务，只由代理发布 `443` 并转发到 `app:3000`。不要直接恢复明文宿主机端口或把端口改为 `0.0.0.0`。

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
3. 如需生成索引库，运行 `docker compose run --rm api index --secret-key-file /run/secrets/paper_scanner_key --file <csv>` 或把现有 `.sqlite` 放入 `data/index/`

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
