# Docker 部署

本文档是服务端 `apps/server/Dockerfile` 与 `docker-compose.yml` 的部署操作手册。命令参数见 [CLI 参考](../reference/cli.md)，安全边界见[安全说明](security.md)。

首次部署按下文的初始化步骤执行；已有数据先阅读备份与恢复要求。示例在仓库根目录的 Bash 中执行，所需工具包括 Docker Compose、OpenSSL 和 curl。

## 服务拓扑

```text
browser/API/MCP -> 127.0.0.1:8000 -> gateway (Caddy)
                                    |-- server:8000 (Go API + scheduler)
                                    `-- client-web:8080 (Wails Server Build + Vite)
server -> ./data:/app/data + litradar_key Compose secret
```

Compose 项目名为 `litradar`，服务为 `server`、`client-web`、`gateway`。只有网关发布宿主端口；客户端仅连接内部 frontend 网络，没有数据卷或密钥。server 连接 backend 网络并保留访问上游所需的出站连接。各服务的运行账号和文件系统限制见[容器限制](#容器限制)。

| 服务       | 构建                                                                                       | 镜像                                         | 持久化                    |
| ---------- | ------------------------------------------------------------------------------------------ | -------------------------------------------- | ------------------------- |
| server     | `apps/server/Dockerfile`                                                                   | `ghcr.io/qianfuv/litradar-server:latest`     | `/app/data`，只读部署密钥 |
| client-web | `apps/client/build/docker/Dockerfile.server`，上下文为仓库根目录（专用 Dockerfile ignore） | `ghcr.io/qianfuv/litradar-client-web:latest` | 无                        |
| gateway    | digest 固定的 Caddy 2.11.2 Alpine                                                          | 见 Compose                                   | 只读 `Caddyfile`          |

server 镜像在 0.5.0 之后由 `ghcr.io/qianfuv/litradar` 更名为 `ghcr.io/qianfuv/litradar-server`，旧名称不再发布新版本。从旧名称升级时，把自己 Compose 文件中 server 的 `image` 改为新名称（或改用仓库中的 `docker-compose.yml`），再执行 `docker compose pull` 和 `docker compose up -d`。

根 `Caddyfile` 路由：

| 路径                                                                                                  | 目标              |
| ----------------------------------------------------------------------------------------------------- | ----------------- |
| `/api`、`/api/*`、`/mcp`、`/mcp/*`、`/docs`、`/docs/*`、`/openapi.json`、`/openapi.yaml`、`/health/*` | server            |
| `/wails`、`/wails/*`                                                                                  | 网关直接返回 404  |
| 其他，包括 `/`、`/login`                                                                              | client-web 的 SPA |

Wails Server Build 的 runtime 端点未经认证；Web host 不注册任何服务，因此网关禁止整个 runtime 命名空间。网关保留 Authorization、Cookie、Origin、Host，覆盖 X-Forwarded-For 为直连地址并移除 Forwarded；MCP 使用 `flush_interval -1`，保留流式响应。[Caddy 官方说明](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy)介绍转发头和缓冲行为。默认入口是 loopback HTTP，自动 HTTPS 和配置持久化关闭。

## 镜像内容与源码构建

服务端 Dockerfile 使用固定 Ubuntu digest、Go 归档校验和与 CA 包版本，不安装 Node.js，也不构建 UI。`apps/server` 以 `CGO_ENABLED=0` 编译为纯 Go 二进制，构建阶段不安装 C 编译器或交叉编译工具链。服务端镜像包含 Obscura v0.2.4、Poppler、不可变 Meta bundle、许可证和 Go 构建清单。服务端 `/` 与 UI 路径返回 404，`/docs`、`/openapi.json`、API、MCP、健康检查可用。

客户端先在根目录构建 Vite，再由独立 Dockerfile 将 `apps/client/frontend/dist` 嵌入 linux/amd64 Wails Web host，并放入 distroless 镜像。两个应用运行镜像均不需要 Node.js。

```bash
pnpm install --frozen-lockfile
pnpm build
docker compose build
docker compose up -d
```

`server` 在同一进程中监管 HTTP 与持久化调度，按需使用同一可执行文件启动任务子进程；SIGINT/SIGTERM 协调关闭并等待子进程。Web 客户端不运行索引或调度。

## 首次部署

### 1. 目录权限和密钥

```bash
mkdir -p data secrets
if [ ! -e secrets/litradar.key ]; then
  (umask 077; openssl rand -out secrets/litradar.key 32)
fi
```

仅在首次部署且尚无密钥时生成新文件；已有数据库继续使用匹配密钥。Linux 原生 Docker Engine 还需让固定容器账号读写专用数据目录、读取密钥：

```bash
sudo chown -R 10001:10001 data
sudo chown 10001:10001 secrets/litradar.key
sudo chmod 600 secrets/litradar.key
```

Docker Desktop for macOS/Windows 通常由虚拟化层转换 bind mount 权限，不应照搬 Linux `chown`。拉取镜像后，可用只读权限检查确认容器账号能读取密钥并写入数据目录：

```bash
docker compose run --rm --no-deps --entrypoint sh server \
  -c 'test -r /run/secrets/litradar_key && test -w /app/data'
```

命令应退出 0；它只检查权限，不生成密钥或业务数据。

已有明文集成凭据的 `data/auth.sqlite` 必须在停机和备份后先执行显式密文迁移，见[安全说明](security.md#旧明文迁移)。

### 2. 拉取和启动

```bash
docker compose pull
docker compose up -d --remove-orphans
docker compose ps
```

镜像更新后，先按[备份与恢复](backup.md)创建并验证备份，再运行 `docker compose pull` 和 `docker compose up -d --remove-orphans`。新镜像启动时可能把旧格式内容库就地迁移到 v10（见[旧分词器索引与中文检索](#旧分词器索引与中文检索)），不支持当前 v10 的旧镜像随后无法直接打开这些文件。

需要验证当前源码时改为本地构建：

```bash
docker buildx build -f apps/server/Dockerfile --platform linux/amd64 --load -t ghcr.io/qianfuv/litradar-server:latest .
docker compose up -d --remove-orphans
```

`docker compose config --services` 应输出 `server`、`client-web`、`gateway`。

### 3. 初始化管理员

在交互式 Bash 中安全读取至少 12 个 Unicode 字符的密码，再传给 stdin：

```bash
IFS= read -r -s -p 'Admin password: ' ADMIN_PASSWORD
printf '\n'
printf '%s\n' "$ADMIN_PASSWORD" |
  docker compose run --rm -T server admin bootstrap \
    --username admin \
    --password-stdin
unset ADMIN_PASSWORD
```

容器入口已经是 `litradar`，因此 `admin` 是首个参数。bootstrap 的约束见[安全说明](security.md#首个管理员)。

### 4. 运行配置

登录 `http://localhost:8000`，在管理员“运行配置”页面设置：

- Scholarly 索引必需的 Crossref 联系邮箱、OpenAlex 和 Semantic Scholar 密钥池
- 每个 CSV/内容库的索引 Provider，以及摘要页/全文的默认和 catalog 覆盖顺序
- 跨源 CORS
- MCP Host/Origin
- Secure Cookie
- 日志格式和严格 filter

字段、默认值和秘密语义见[运行配置参考](../reference/configuration.md)。

### 5. 构建索引

CNKI 示例：

```bash
docker compose run --rm server index \
  --secret-key-file /run/secrets/litradar_key \
  --file chinese_journals.csv \
  --update
```

配置上述 Scholarly 凭据后可把文件替换为 `english_journals.csv` 或 `ccf_computer_journals.csv`。已有索引库也可直接放入宿主机 `data/index/`。

## 索引中断恢复

每条实时索引或更新命令先取得项目级租约 `index_batch_lease`（`data/index-control/index-batches.sqlite`），再为当前目录取得 catalog 控制库中的 `provider_leases` 次级租约；父进程定期续期，硬终止后租约最长 300 秒过期。普通上游、worker 或清单错误会保留 active batch 和待发布事件并释放所有权。租约、batch 兼容条件和增量边界见 [CLI 参考](../reference/cli.md)与[数据库参考](../reference/database.md)。

恢复时按以下顺序操作：

1. 确认旧容器、计划任务子进程和画像容器已经停止；不要通过删除 `index_batch_lease` 或 catalog `provider_leases` 绕过所有权检查。
2. 停止常驻服务并完成离线、已验证的当前数据备份。部署密钥必须继续留在 Compose secret 中，不得复制到备份或日志。
3. 普通失败可立即重跑同一命令；硬终止必须等到旧租约过期。未过期时的明确所有者错误表示旧运行仍受保护，不是可忽略的重试提示。
4. 需要恢复 changes JSON 时，用与原 batch 兼容的目录选择、模式和 notify 选项重跑 `--update`，让默认 `--resume` 继续同一 active batch。存在已发布或可能已发布 manifest 时不要用 `--no-resume` 丢弃 handoff。
5. 成功后确认命令退出 0、changes JSON 可解析、batch 历史进入 `completed`、`index_batch_lease` 与 catalog `provider_leases` 都没有活动所有者，再启动服务并检查 `/health/live`、`/health/ready` 和 `/`。

## 数据和秘密

| 宿主机或镜像路径         | 容器路径                    | 说明                          |
| ------------------------ | --------------------------- | ----------------------------- |
| `./data`                 | `/app/data`                 | 唯一持久可写业务挂载          |
| `./secrets/litradar.key` | `/run/secrets/litradar_key` | Compose secret，只读          |
| 镜像官方 Meta bundle     | `/usr/share/litradar/meta`  | 不可变源；不在 `/app/data` 内 |

`LITRADAR_DATA_DIR` 和 `LITRADAR_SECRET_KEY_FILE` 可把上表宿主机路径改为专用数据目录和密钥文件。

Crossref 可丢弃工作集固定在 `/app/data/index-work/scholarly/`，随现有 data 挂载持久化，不需要额外挂载或 `/tmp` 空间，也不进入备份；容量和缓存规则见[数据库参考](../reference/database.md)。容量不足会明确失败并保留正式内容。不要在运行中手工清空工作目录，也不要用删除控制状态代替容量诊断。

### Meta bundle 与持久卷

Docker bind mount 和 Kubernetes PVC 会遮蔽挂载点中的镜像层内容，不会执行目录合并。LitRadar 因此把官方源与持久副本分开：Dockerfile 固定把 bundle 复制到 `/usr/share/litradar/meta`，应用依次查找 `/usr/share/litradar/meta/bundle-manifest.json` 和可执行文件旁的 `assets/meta/bundle-manifest.json`，使用第一个存在的 bundle，于数据库迁移后把清单允许的更新同步到 `/app/data/meta`。镜像中只有前者存在。这些位置不是环境变量、CLI 或管理员可覆盖项，也不能改指向可写的持久目录。

`serve` 和普通 `index` 都会在读取期刊目录前执行一次准备。调度器启动的普通索引子进程也经过这个入口；多进程索引的内部 worker 不重复执行。准备结果产生 `event=storage.managed_meta.prepared component=storage` 的聚合事件，索引 stdout JSON 保持不变。

| 卷中状态                     | 启动结果                                   |
| ---------------------------- | ------------------------------------------ |
| 空 PVC 或缺少某个官方文件    | 创建当前官方副本并记录状态                 |
| 已知旧版官方文件             | 原子升级并更新状态                         |
| 已经是当前官方内容但没有状态 | 接管状态，不重写内容                       |
| 上次受管内容未被用户修改     | 新 bundle 到来时原子升级                   |
| 同名文件已自定义或内容未知   | 保留原文件，记录 `customized` 诊断，不覆盖 |
| 清单之外的用户文件           | 保持不变                                   |

受管状态存储在 `data/auth.sqlite` 的 `managed_meta_catalogs`。若卷中记录的 bundle 版本高于当前镜像，旧镜像会在写入前以 downgrade 错误退出；镜像回滚必须使用兼容版本或经过验证的整套备份恢复，不能用强制复制绕过。替换或状态提交失败会回滚本轮文件变更。

除征稿辅助程序路径和 `litradar index` 的 `LITRADAR_CNKI_CAPTCHA_TOKEN` 单次探测回退外，应用的监听、数据路径和秘密边界由 CLI 参数与只读密钥文件提供。浏览器同源 API、开发代理、Meta bundle、日志和父子进程关联分别由固定代码路径、数据库运行设置或隐藏内部参数负责，不接受任意环境变量覆盖；完整配置来源见[运行配置](../reference/configuration.md)。

新清单不再列出的退役或改名文件不会自动删除。先用当前二进制创建并验证 v2 备份，确认没有保存的任务或手工命令引用旧 CSV，再逐个手工删除明确识别的文件。不要批量删除未知文件，也不要用 `cp -f` 覆盖自定义目录。

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
curl --fail http://localhost:8000/docs
curl --fail http://localhost:8000/openapi.json
docker compose ps
```

`/health/live` 表示应用事件循环存活。`/health/ready` 只有在内嵌调度的持久化心跳处于 90 秒健康窗口内时才返回 `200`，否则返回 `503`。server 健康检查只请求 readiness；网关页面需单独探测。Docker unhealthy 本身不会杀死仍在运行的进程；`restart: unless-stopped` 处理进程退出和 daemon 重启。

`/mcp` 位于 `http://localhost:8000/mcp`；未认证请求预期返回 `401`，实际客户端应携带访问令牌或会话 Cookie。

## 容器限制

三个服务都使用 `read_only: true`、`restart: unless-stopped` 和 `no-new-privileges:true`，其余限制按服务区分：

| 服务       | 运行账号               | capabilities                        | 可写位置                                                      | 其他                                    |
| ---------- | ---------------------- | ----------------------------------- | ------------------------------------------------------------- | --------------------------------------- |
| server     | UID/GID `10001:10001`  | 全部丢弃                            | `/app/data`；`/tmp` 为 `noexec,nosuid,nodev`、64 MiB 的 tmpfs | 只读密钥挂载；镜像内 readiness 健康检查 |
| client-web | distroless `nonroot`   | 全部丢弃                            | 无                                                            | 不挂载业务数据或部署密钥                |
| gateway    | Caddy 官方镜像默认账号 | 全部丢弃，仅保留 `NET_BIND_SERVICE` | 无                                                            | 只读 `Caddyfile`                        |

网关保留 `NET_BIND_SERVICE`，因为官方 Caddy 二进制带该文件 capability；从 bounding set 移除后，即使监听 8000 也会在执行时返回 EPERM。除 `/app/data` 外没有持久写路径。页面由独立 client-web 提供，server 不读 UI 资源。不要通过 root 容器、开放整个宿主机目录或挂载 Docker socket 解决权限问题。

Compose 默认不设置 cgroup 内存上限。需要限定部署预算时，可在 Compose 覆盖文件中设置 `mem_limit`，或向 `docker run` 传入 `--memory`。服务与调度子进程共享容器 cgroup；内存测量见[性能剖析](../guides/profiling.md)。

上游响应在透明解压后仍受各自限制：CNKI/ZJLib 为 2 MiB，JFBYM 为 256 KiB，Scholarly 为 16 MiB，PDF 为 32 MiB。`Content-Length` 只用于提前检查，解压后的流仍按 `limit + 1` 读取。这些响应上限、有界队列和持久页确认机制不依赖容器是否设置内存上限。

## 日志收集与轮转

应用把 JSON Lines 写入 `stderr`，不在 `/app` 创建日志文件；Compose 使用 Docker `local` 驱动，每个容器在压缩前最多约 50 MiB。查询命令、保留边界和事故流程见[日志运维](logging.md#docker-保留策略)。

## 通过 HTTPS 提供访问

### 1. 确认镜像发布

CI 在 `VERSION` 递增且后端、前端和客户端质量检查全部通过后，发布 `ghcr.io/qianfuv/litradar-server` 与 `ghcr.io/qianfuv/litradar-client-web` 的 `v<version>` 和 `latest`（仅 linux/amd64）。部署前确认对应 GitHub Release 已发布；可复现部署优先使用 `v<version>`，`latest` 是可变 tag。流程见[版本发布](releases.md)。

### 2. 配置 HTTPS 访问

先通过 loopback 完成管理员 bootstrap。在管理员运行配置设置 `public_base_url=https://papers.example.org`、`secure_cookies=true`、`trusted_proxy_cidrs=172.30.42.0/24`（默认 backend 网段）和 `mcp_allowed_hosts`（包含 `papers.example.org`）。网络、Cookie 与 MCP 设置保存后重启 server；字段语义见[运行配置参考](../reference/configuration.md)。不要用 `0.0.0.0/0` 代替准确可信网络。

默认 backend 子网可用 `LITRADAR_BACKEND_SUBNET` 改为不冲突的私网 CIDR，同时更新 `trusted_proxy_cidrs`。`LITRADAR_BIND_ADDRESS` 和 `LITRADAR_PORT` 控制网关发布地址（默认 `127.0.0.1:8000`）。client-web 使用 `WAILS_SERVER_HOST`/`WAILS_SERVER_PORT`；客户端 HSTS 使用 `LITRADAR_CLIENT_HSTS_MAX_AGE`。

生产 TLS 建议直接在同一 Caddy 中使用域名站点，移除 `auto_https off`，发布 80/443，并为 Caddy 的证书状态配置独立持久存储；业务数据和部署密钥仍仅授予 server。保留原有 handle 路由、MCP `flush_interval` 和请求头边界。启用 HTTPS 后将 client-web 的 `LITRADAR_CLIENT_HSTS_MAX_AGE` 设为 `31536000`，server 命令增加 `--require-secure-cookies`。

默认 Caddy 将 X-Forwarded-For 设为直连地址。如果额外接入外层 TLS/CDN 代理，必须显式配置准确可信代理链和客户端地址解析，再将经过验证的客户端地址传给 server；不能盲目信任外来 Forwarded/X-Forwarded-For。默认 loopback 示例不会自动信任外层代理。

```bash
docker compose config --quiet
docker compose config --services
curl --fail http://localhost:8000/health/ready
curl --fail http://localhost:8000/docs
curl --fail http://localhost:8000/login
# Expected: 404
curl -o /dev/null -w '%{http_code}\n' http://localhost:8000/wails/runtime
```

可用 `node tests/compose-smoke.mjs <server-image> <client-image>` 对临时项目执行真实三服务、Cookie/Bearer 登录与路由验证；脚本只创建合成密钥与专用卷，并在结束时清理。

应用 URL 校验不能替代网络策略。容器或主机 egress ACL 只应允许管理员批准的 AI/PushPlus 目标、DNS 和 TLS 基础设施，并显式阻断 loopback、RFC1918、link-local、元数据地址与内部服务网段。多实例部署还必须在反向代理/网关使用共享认证限流；应用内单实例 bucket 不是跨实例协调器。

## 按需命令

维护和作业命令复用相同镜像与入口，不启动 HTTP 或调度循环：

```bash
docker compose run --rm server --help
docker compose run --rm server openapi
docker compose run --rm server scheduler validate \
  --secret-key-file /run/secrets/litradar_key
```

## HTTP MCP

MCP 端点内置于统一应用的 `/mcp`，不需要单独服务：

- 桌面/命令行客户端使用 `Authorization: Bearer <access_token>`
- 同源浏览器可使用 `litradar_session` Cookie
- 非 loopback 域名或反向代理必须加入 `mcp_allowed_hosts`
- 浏览器跨源直连时再配置 `mcp_allowed_origins`

## 备份和恢复

通过独立 `/backups` bind mount 运行 `litradar admin backup`，不要把备份输出写入 `/app/data`。恢复前必须停止 `server` 服务并等待活动心跳过期。完整流程和 Docker 命令见[备份与恢复](backup.md)。

## 离线索引存储优化

`admin index optimize-storage` 离线整目录重建 `data/index` 为 v10 并压缩，必须在停机窗口运行。前置条件、空间要求、输出核对和回滚点见[备份与恢复](backup.md#为离线索引优化准备回滚点)，命令语义、输出字段和错误码见 [CLI 参考](../reference/cli.md)。不支持 v10 的旧镜像不能打开优化后的文件。

先停止服务和所有一次性 `index`/投递容器，等待心跳和租约过期，再通过独立 `/backups` bind mount 创建并验证备份：

```bash
mkdir -p backups
sudo chown 10001:10001 backups

docker compose stop server

docker compose run --rm --no-deps \
  -v "$PWD/backups:/backups:rw" \
  server admin backup create \
    --project-root /app \
    --output /backups/litradar-before-index-v10 \
    --include-indexes \
    --include-push-state

docker compose run --rm --no-deps \
  -v "$PWD/backups:/backups:ro" \
  server admin backup verify \
    --backup /backups/litradar-before-index-v10
```

确认空间后运行当前镜像中的维护命令，并保存完整 stdout JSON 和容器日志：

```bash
docker compose run --rm --no-deps \
  -v "$PWD/backups:/backups:ro" \
  server admin index optimize-storage \
    --project-root /app \
    --confirm-index-maintenance
```

失败若返回非空 `error.recovery_paths`，不要删除 marker、staging 或 rollback，也不要启动服务。成功退出后启动同一镜像并做 smoke：

```bash
docker compose up -d --remove-orphans
curl --fail http://localhost:8000/
curl --fail http://localhost:8000/health/live
curl --fail http://localhost:8000/health/ready
curl --fail http://localhost:8000/openapi.json
```

还要用固定查询核对检索、过滤、游标分页和文章详情。需要降级时，先停止当前服务，再按[离线恢复](backup.md#离线恢复)恢复受目标镜像支持的优化前备份。

## 排障

### Web 可访问但没有检索结果

1. 运行 `docker compose logs --since 30m server`，按 `index.*`、`source.*` 和 `error_kind` 查询。
2. 确认宿主机 `data/index/*.sqlite` 存在。
3. 确认 bind mount 权限。
4. 按 CLI 参考运行单个 CSV 索引。

### readiness 返回 503

1. 运行 `docker compose logs --since 30m server`，查找 `scheduler.tick.*`、`service.component.failed` 或安全错误分类。
2. 确认应用可以写入同一个 `data/auth.sqlite`。
3. 确认没有长时间阻塞的任务；调度心跳健康窗口为 90 秒。
4. 用 `docker compose run --rm server scheduler validate --secret-key-file /run/secrets/litradar_key` 验证已保存任务。

### 旧分词器索引与中文检索

检索分词在 Go 中完成，镜像不包含 Simple、`libsimple.so` 或任何 SQLite 分词扩展；Obscura 与 Poppler 是独立辅助程序。普通启动的预检按 `article_search` 声明判断：仍为 `unicode61`（v6/v7/v8）或 `simple 0`（旧 v9/v10）的内容库会在一个事务内就地迁移到 v10 并重建检索投影，失败时回滚，原声明保持不变。备份恢复出的旧快照也按此迁移。完成后复核版本、FTS 定义和代表性查询；不得降低版本号，也不要向运行镜像复制历史 DLL/SO。分词规则和迁移路径见[数据库参考](../reference/database.md#article_search)。

### 通知没有结果

检查 `.changes.json`、用户偏好、AI/PushPlus 凭据，以及认证库中的 `delivery_runs`、`delivery_run_items`、`delivery_dedupe` 和 `delivery_leases`。不要通过修改旧状态 JSON 来清除 busy/unknown；详见[通知与追踪](../guides/notifications.md)。
