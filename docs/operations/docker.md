# Docker 部署

本文档是 Docker 镜像和根目录 `docker-compose.yml` 的部署 runbook。命令参数见 [CLI 参考](../reference/cli.md)，安全边界见[安全说明](security.md)。

## 服务拓扑

```text
browser
  |
  +-- 127.0.0.1:3000 --> app (Next.js)
  |
  +-- 127.0.0.1:8000 --> api (Rust)
                              ^
                              |
                         worker sidecar

api + worker --> ./data:/app/data
api + worker --> paper_scanner_key Compose secret
```

Compose 默认只发布宿主机 loopback，不直接暴露到局域网或公网。

## 服务

### `api`

| 项目       | 值                                                                                                    |
| ---------- | ----------------------------------------------------------------------------------------------------- |
| 构建上下文 | 仓库根目录                                                                                            |
| 镜像       | `ghcr.io/qianfuv/paper-scanner-api:latest`                                                            |
| 命令       | `api --host 0.0.0.0 --port 8000 --project-root /app --secret-key-file /run/secrets/paper_scanner_key` |
| 宿主机端口 | `127.0.0.1:8000:8000`                                                                                 |
| 可写数据   | `./data:/app/data:rw`                                                                                 |
| 运行用户   | `paper`，UID/GID `10001:10001`                                                                        |
| 健康检查   | `GET /api/health`                                                                                     |

### `worker`

| 项目     | 值                                                                                                  |
| -------- | --------------------------------------------------------------------------------------------------- |
| 镜像     | 与 `api` 相同                                                                                       |
| 命令     | `worker --project-root /app --secret-key-file /run/secrets/paper_scanner_key --interval-seconds 30` |
| 可写数据 | `./data:/app/data:rw`                                                                               |
| 依赖     | `api` 健康                                                                                          |
| 健康检查 | 通过 `api:8000/api/health/worker` 读取持久心跳                                                      |

worker 每轮更新调度游标和心跳。`/api/health/worker` 在最近 90 秒没有 worker 心跳时返回 `503`。

### `app`

| 项目       | 值                                               |
| ---------- | ------------------------------------------------ |
| 构建上下文 | `app/`                                           |
| 镜像       | `ghcr.io/qianfuv/paper-scanner-app:latest`       |
| 宿主机端口 | `127.0.0.1:3000:3000`                            |
| 环境       | `HOSTNAME=0.0.0.0`                               |
| 运行用户   | Node 镜像内置 `node` 用户                        |
| 依赖       | `api` 健康                                       |
| 健康检查   | 请求前端 `/api/health`，同时验证 Next.js rewrite |

`app/Dockerfile` 的 `INTERNAL_API_URL` 构建参数默认是 `http://api:8000`，所以浏览器使用同源 `/api/*` 即可访问 Rust API。

## 镜像内容

后端镜像分两阶段：

1. `rust:1.96-bookworm` 构建 release 二进制。
2. `debian:bookworm-slim` 复制 `admin`、`api`、`ps-api`、`index`、`notify`、`push`、`scheduler`、`worker`、Linux `simple` 扩展和 `data/meta/`。

运行层安装 CA 证书和 `curl`，随后切换到固定非 root 用户。镜像不包含 Python 运行时。

前端镜像使用 Node.js 24 Alpine：依赖、构建和 standalone 运行分层，最终由非 root `node` 用户执行 `server.js`。

根后端 Dockerfile 的默认 CMD 没有部署密钥参数，不能直接用于生产启动。Compose 显式覆盖命令；自行 `docker run` 时必须只读挂载密钥并传入 `--secret-key-file`。

## 首次部署

### 1. 目录权限和密钥

```bash
mkdir -p secrets
openssl rand -out secrets/paper-scanner.key 32
chmod 600 secrets/paper-scanner.key
```

Linux 原生 Docker Engine：

```bash
sudo chown -R 10001:10001 data
```

Docker Desktop for macOS/Windows 通常由虚拟化层转换 bind mount 权限，不应照搬 Linux `chown`。

已有明文集成凭据的 `data/auth.sqlite` 必须在停机和备份后先执行显式密文迁移，见[安全说明](security.md)。

### 2. 构建和启动

```bash
docker compose up -d --build
docker compose ps
```

### 3. 初始化管理员

```bash
printf '%s\n' "$ADMIN_PASSWORD" |
  docker compose run --rm -T api admin bootstrap \
    --username admin \
    --password-stdin
```

`api` 是 Compose 服务名，`admin` 是容器内执行的维护命令。用户表非空时 bootstrap 会拒绝。

### 4. 运行配置

登录 `http://localhost:3000`，在管理员“运行配置”页面设置：

- scholarly 索引需要的 OpenAlex 和 Semantic Scholar key
- 可选 Crossref 联系邮箱
- 跨源 CORS
- MCP Host/Origin
- Secure Cookie

字段、默认值和秘密语义见[运行配置参考](../reference/configuration.md)。

### 5. 构建索引

CNKI 示例：

```bash
docker compose run --rm api index \
  --secret-key-file /run/secrets/paper_scanner_key \
  --file chinese_journals.csv \
  --update
```

配置 scholarly key 后可把文件替换为 `english_journals.csv` 或 `ccf_computer_journals.csv`。已有索引库也可直接放入宿主机 `data/index/`。

## 数据和秘密

| 宿主机路径                    | 容器路径                         | 说明                                |
| ----------------------------- | -------------------------------- | ----------------------------------- |
| `./data`                      | `/app/data`                      | API/worker 唯一持久可写业务挂载     |
| `./secrets/paper-scanner.key` | `/run/secrets/paper_scanner_key` | Compose secret，只读                |
| 镜像内 `data/meta`            | `/app/data/meta` 的初始镜像内容  | bind mount 后以宿主机 `./data` 为准 |

重要数据包括：

- `data/meta/*.csv`
- `data/index/*.sqlite`
- `data/auth.sqlite`
- `data/push_state/`
- `data/folder_push_state/`

部署密钥不在 `./data`，也不应和数据备份放进同一归档。

## 健康检查

```bash
curl --fail http://localhost:8000/api/health
curl --fail http://localhost:8000/api/health/worker
curl --fail http://localhost:3000/api/health
docker compose ps
```

worker 首轮写入心跳前可能暂时 unhealthy；停止超过 90 秒后会再次变为 unhealthy。Docker unhealthy 本身不会杀死仍在运行的进程，`restart: unless-stopped` 处理的是进程退出和 daemon 重启。

## 容器限制

三个服务共同启用：

- `read_only: true`
- `restart: unless-stopped`
- `cap_drop: [ALL]`
- `no-new-privileges:true`
- 带 `noexec,nosuid` 的 `/tmp` tmpfs
- 明确健康检查

前端额外挂载只允许 `node` 用户写入的 `/app/.next/cache` tmpfs。后端除 `/app/data` 外没有持久写路径。

## 公网部署

生产环境应：

1. 在管理员运行配置中设置 `secure_cookies=true`。
2. 停止服务。
3. 为 API 命令增加 `--require-secure-cookies`。
4. 移除 API 和 app 的宿主机端口发布。
5. 在同一网络加入只发布 HTTPS `443` 的反向代理。
6. 配置准确的 CORS Origin、MCP Host/Origin 和代理层共享限流。

示例覆盖文件：

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

`!reset` 需要 Docker Compose 2.24.4 或更新版本：

```bash
docker compose \
  -f docker-compose.yml \
  -f compose.production.yaml \
  up -d
```

不要用 `0.0.0.0` 宿主机端口替代反向代理。

## HTTP MCP

MCP 端点内置于 API 的 `/mcp`，不需要单独服务：

- 桌面/命令行客户端使用 `Authorization: Bearer <access_token>`
- 同源浏览器可使用 `ps_session` Cookie
- 非 loopback 域名或反向代理必须加入 `mcp_allowed_hosts`
- 浏览器跨源直连时再配置 `mcp_allowed_origins`

## 备份和恢复

通过独立 `/backups` bind mount 运行 `admin backup`，不要把备份输出写入 `/app/data`。恢复前必须停止 `app`、`api`、`worker` 并等待活动心跳过期。完整流程见[备份与恢复](backup.md)。

## 排障

### 前端可访问但没有检索结果

1. `docker compose logs api`
2. 确认宿主机 `data/index/*.sqlite` 存在
3. 确认 bind mount 权限
4. 按 CLI 参考运行单个 CSV 索引

### worker unhealthy

1. 等待首次 30 秒轮询
2. 查看 `docker compose logs worker`
3. 确认 worker 和 API 使用同一个 `data/auth.sqlite` 与密钥
4. 检查密文验证或迁移错误

### 中文检索质量异常

确认镜像中存在 `libs/simple-linux/libsimple-linux-ubuntu-latest/libsimple.so`。扩展加载失败时系统会退回默认 FTS5 tokenizer，不会自动重建既有 FTS 表。

### 通知没有结果

检查变更清单、用户偏好、AI/PushPlus 凭据和正确的状态目录，详见[通知与追踪](../guides/notifications.md)。
