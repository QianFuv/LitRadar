# Docker 部署

本文档是根目录 `Dockerfile` 与 `docker-compose.yml` 的部署 runbook。命令参数见 [CLI 参考](../reference/cli.md)，安全边界见[安全说明](security.md)。

## 服务拓扑

```text
browser / API / MCP client
           |
           v
127.0.0.1:8000 -> litradar container
                    `-- litradar serve (one long-running process)
                          |-- static Web / REST / Swagger / OpenAPI / MCP
                          |-- embedded persistent scheduler
                          `-- transient same-binary job children when due

litradar -> ./data:/app/data
litradar -> litradar_key Compose secret
litradar -> ghcr.io/qianfuv/litradar:latest
```

Compose 项目名固定为 `litradar`，并且只声明一个同名服务。HTTP 和调度共享一个应用生命周期；没有第二个常驻容器。默认只把 8000 端口发布到宿主机 loopback，不直接暴露到局域网或公网。

## 服务契约

| 项目       | 值                                                                                                 |
| ---------- | -------------------------------------------------------------------------------------------------- |
| 服务名     | `litradar`                                                                                         |
| 构建上下文 | 仓库根目录                                                                                         |
| 镜像       | `ghcr.io/qianfuv/litradar:latest`                                                                  |
| 入口       | `litradar`                                                                                         |
| 默认命令   | `serve --host 0.0.0.0 --port 8000 --project-root /app --secret-key-file /run/secrets/litradar_key` |
| 宿主机端口 | `127.0.0.1:8000:8000`                                                                              |
| 可写数据   | `./data:/app/data:rw`                                                                              |
| 运行用户   | `litradar`，UID/GID `10001:10001`                                                                  |
| 健康检查   | `GET /health/ready` 后再请求根 Web 文档 `GET /`                                                    |

`litradar serve` 在绑定端口前完成数据库迁移、密钥验证和 HTTP 准备，然后立即执行第一个调度 tick。默认每 30 秒再次检查计划任务。调度任务通过同一 `/usr/local/bin/litradar` 启动短生命周期的 `index`、`notify` 或 `push` 子进程；这些子进程不是 Compose 服务。

SIGINT/SIGTERM 会协调关闭 HTTP 与调度组件。若任务子进程正在运行，应用会终止并等待它，把运行状态保存为 `cancelled`，且不再启动该任务的剩余步骤。HTTP、心跳或调度组件意外失败会关闭整个进程并返回非零状态。

## 镜像内容

根 Dockerfile 包含以下构建阶段：

1. Node.js 24 Alpine 只复制 `app/package.json` 和 lockfile，使用缓存安装依赖。
2. 独立前端构建阶段复制 `app/` 源码，生成 `out/`，并为 HTML、CSS、JavaScript、JSON、SVG、TXT、XML 和 source map 保留原文件及确定性 gzip 兄弟文件。
3. `rust:1.96-bookworm` 只构建 release `litradar` 目标。
4. `debian:trixie-slim` 只复制 `/usr/local/bin/litradar`、Linux `simple` 扩展、`data/meta/` 和 `/app/web`。

运行层安装 CA 证书、`curl` 和扩展所需的 `libstdc++6`，随后切换到固定非 root 用户 `litradar`。Trixie 满足镜像内 `libsimple.so` 所需的 `GLIBC_2.38` 与 `GLIBCXX_3.4.32`。最终镜像不包含其他 LitRadar 可执行文件、Node.js、Next.js standalone、`server.js` 或 Python 运行时。默认 `ENTRYPOINT` 与 `CMD` 已包含应用、`serve` 子命令和密钥路径，因此 Compose 不覆盖命令；自行使用 `docker run` 时仍必须把 32 字节密钥只读挂载到该路径。

支持 gzip 的客户端会直接收到预压缩文件，不支持的客户端仍收到原文件。`/_next/static/*` 成功响应使用长期 public immutable 缓存；页面、导航 payload 和导出的 404 使用 `no-cache`；认证/API 的私有缓存边界不因此放宽。

## 首次部署

### 1. 目录权限和密钥

```bash
mkdir -p secrets
openssl rand -out secrets/litradar.key 32
chmod 600 secrets/litradar.key
```

Linux 原生 Docker Engine：

```bash
sudo chown -R 10001:10001 data
```

Docker Desktop for macOS/Windows 通常由虚拟化层转换 bind mount 权限，不应照搬 Linux `chown`。

已有明文集成凭据的 `data/auth.sqlite` 必须在停机和备份后先执行显式密文迁移，见[安全说明](security.md)。

### 2. 拉取和启动

```bash
docker compose pull
docker compose up -d --remove-orphans
docker compose ps
```

需要验证当前源码时改为本地构建：

```bash
docker compose build litradar
docker compose up -d --remove-orphans
```

`docker compose config --services` 应只输出 `litradar`。

### 3. 初始化管理员

```bash
printf '%s\n' "$ADMIN_PASSWORD" |
  docker compose run --rm -T litradar admin bootstrap \
    --username admin \
    --password-stdin
```

容器入口已经是 `litradar`，因此 `admin` 是首个参数。用户表非空时 bootstrap 会拒绝。

### 4. 运行配置

登录 `http://localhost:8000`，在管理员“运行配置”页面设置：

- scholarly 索引需要的 OpenAlex 和 Semantic Scholar key
- 可选 Crossref 联系邮箱
- 跨源 CORS
- MCP Host/Origin
- Secure Cookie

字段、默认值和秘密语义见[运行配置参考](../reference/configuration.md)。

### 5. 构建索引

CNKI 示例：

```bash
docker compose run --rm litradar index \
  --secret-key-file /run/secrets/litradar_key \
  --file chinese_journals.csv \
  --update
```

配置 scholarly key 后可把文件替换为 `english_journals.csv` 或 `ccf_computer_journals.csv`。已有索引库也可直接放入宿主机 `data/index/`。

## 数据和秘密

| 宿主机路径               | 容器路径                        | 说明                                |
| ------------------------ | ------------------------------- | ----------------------------------- |
| `./data`                 | `/app/data`                     | 唯一持久可写业务挂载                |
| `./secrets/litradar.key` | `/run/secrets/litradar_key`     | Compose secret，只读                |
| 镜像内 `data/meta`       | `/app/data/meta` 的初始镜像内容 | bind mount 后以宿主机 `./data` 为准 |

重要数据包括：

- `data/meta/*.csv`
- `data/index/*.sqlite`
- `data/auth.sqlite`
- `data/push_state/`
- `data/folder_push_state/`

部署密钥不在 `./data`，也不应和数据备份放进同一归档。

## 健康检查

```bash
curl --fail http://localhost:8000/
curl --fail http://localhost:8000/health/live
curl --fail http://localhost:8000/health/ready
curl --fail http://localhost:8000/docs/
curl --fail http://localhost:8000/openapi.json
docker compose ps
```

`/health/live` 表示应用事件循环存活。`/health/ready` 只有在内嵌调度的持久化心跳处于 90 秒健康窗口内时才返回 `200`，否则返回 `503`。Compose 健康检查先请求 readiness，再请求根 Web 文档。Docker unhealthy 本身不会杀死仍在运行的进程；`restart: unless-stopped` 处理进程退出和 daemon 重启。

`/mcp` 位于 `http://localhost:8000/mcp`；未认证请求预期返回 `401`，实际客户端应携带访问令牌或会话 Cookie。

## 容器限制

唯一服务启用：

- `read_only: true`
- `restart: unless-stopped`
- `cap_drop: [ALL]`
- `no-new-privileges:true`
- 带 `noexec,nosuid` 的 `/tmp` tmpfs
- 一份数据挂载、密钥挂载和健康检查

除 `/app/data` 外没有持久写路径。`/app/web` 随镜像只读提供，运行时不生成 Next.js cache。不要通过 root 容器、开放整个宿主机目录或挂载 Docker socket 解决权限问题。

## 公网部署

生产环境应：

1. 在管理员运行配置中设置 `secure_cookies=true`。
2. 停止服务。
3. 为 `serve` 命令增加 `--require-secure-cookies`。
4. 移除应用的宿主机端口发布。
5. 在同一网络加入只发布 HTTPS `443` 的反向代理，并把 Web、API、Swagger/OpenAPI 和 MCP 的全部路径转发到 `litradar:8000`。
6. 配置准确的 CORS Origin、MCP Host/Origin 和代理层共享限流。

示例覆盖文件：

```yaml
services:
  litradar:
    ports: !reset []
    command:
      - serve
      - --host
      - 0.0.0.0
      - --port
      - "8000"
      - --project-root
      - /app
      - --secret-key-file
      - /run/secrets/litradar_key
      - --require-secure-cookies
```

`!reset` 需要 Docker Compose 2.24.4 或更新版本：

```bash
docker compose \
  -f docker-compose.yml \
  -f compose.production.yaml \
  up -d --remove-orphans
```

不要用 `0.0.0.0` 宿主机端口替代反向代理。

## 按需命令

维护和作业命令复用相同镜像与入口，不启动 HTTP 或调度循环：

```bash
docker compose run --rm litradar --help
docker compose run --rm litradar openapi
docker compose run --rm litradar scheduler validate \
  --secret-key-file /run/secrets/litradar_key
```

## HTTP MCP

MCP 端点内置于统一应用的 `/mcp`，不需要单独服务：

- 桌面/命令行客户端使用 `Authorization: Bearer <access_token>`
- 同源浏览器可使用 `litradar_session` Cookie
- 非 loopback 域名或反向代理必须加入 `mcp_allowed_hosts`
- 浏览器跨源直连时再配置 `mcp_allowed_origins`

## 备份和恢复

通过独立 `/backups` bind mount 运行 `litradar admin backup`，不要把备份输出写入 `/app/data`。恢复前必须停止唯一的 `litradar` 服务并等待活动心跳过期。完整流程见[备份与恢复](backup.md)。

## 排障

### Web 可访问但没有检索结果

1. 运行 `docker compose logs litradar`。
2. 确认宿主机 `data/index/*.sqlite` 存在。
3. 确认 bind mount 权限。
4. 按 CLI 参考运行单个 CSV 索引。

### readiness 返回 503

1. 运行 `docker compose logs litradar`，查找调度 tick 或密钥验证错误。
2. 确认应用可以写入同一个 `data/auth.sqlite`。
3. 确认没有长时间阻塞的任务；调度心跳健康窗口为 90 秒。
4. 用 `docker compose run --rm litradar scheduler validate --secret-key-file /run/secrets/litradar_key` 验证已保存任务。

### `simple` tokenizer 加载失败

确认镜像中存在 `libs/simple-linux/libsimple-linux-ubuntu-latest/libsimple.so`，并在最终镜像中运行 `ldd` 检查缺失库或符号版本。索引数据库声明 `tokenize='simple'` 时，服务会在迁移或写入前加载扩展并执行最小 FTS 查询；加载路径错误、ABI 不兼容或探测失败都会保留底层 SQLite loader 错误并退出。新索引也要求该扩展，不再静默创建使用默认 tokenizer 的 FTS 表。已有使用默认 tokenizer 的数据库保持原定义，不会被隐式重建。

### 通知没有结果

检查变更清单、用户偏好、AI/PushPlus 凭据和正确的状态目录，详见[通知与追踪](../guides/notifications.md)。
