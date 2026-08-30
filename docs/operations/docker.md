# Docker 部署

本文档是根目录 `Dockerfile`、本地 `docker-compose.yml` 与生产覆盖文件 `compose.production.yaml` 的部署 runbook。命令参数见 [CLI 参考](../reference/cli.md)，安全边界见[安全说明](security.md)。

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
local only -> ghcr.io/qianfuv/litradar:latest or local build
production -> ghcr.io/qianfuv/litradar@sha256:<verified digest>
```

`docker-compose.yml` 是开发和 loopback 单机配置，Compose 项目名为 `litradar`，并且只声明一个同名服务。HTTP 和调度共享一个应用生命周期；没有第二个常驻容器。默认只把 8000 端口发布到宿主机 loopback，不直接暴露到局域网或公网。

公网部署必须同时加载 `compose.production.yaml`。该覆盖文件删除本地 build 和宿主机端口，只接受 `repository@sha256:<64 hex>` 形式的镜像，强制拉取并以 `--require-secure-cookies` 启动。`latest`、提交 tag 和其他 source tag 只能用于发现或本地便利，不能成为生产配置的运行输入。

## 服务契约

| 项目       | 值                                                                                                 |
| ---------- | -------------------------------------------------------------------------------------------------- |
| 服务名     | `litradar`                                                                                         |
| 构建上下文 | 仓库根目录                                                                                         |
| 本地镜像   | `ghcr.io/qianfuv/litradar:latest` 或当前源码 build                                                 |
| 生产镜像   | `${LITRADAR_IMAGE_REPOSITORY}@sha256:${LITRADAR_IMAGE_DIGEST}`                                     |
| 入口       | `litradar`                                                                                         |
| 默认命令   | `serve --host 0.0.0.0 --port 8000 --project-root /app --secret-key-file /run/secrets/litradar_key` |
| 宿主机端口 | `127.0.0.1:8000:8000`                                                                              |
| 可写数据   | `./data:/app/data:rw`                                                                              |
| 运行用户   | 固定 UID/GID `10001:10001`                                                                         |
| 健康检查   | `GET /health/ready` 后再请求根 Web 文档 `GET /`                                                    |
| 内存上限   | 160 MiB，覆盖服务进程及同 cgroup 的计划任务子进程                                                  |
| 日志       | `local` 驱动；每容器五个 10 MiB 文件，启用压缩                                                     |

`litradar serve` 在绑定端口前依次完成数据库迁移、持久 Meta 准备、密钥验证、运行设置加载和 HTTP 准备，然后立即执行第一个调度 tick。默认每 30 秒再次检查计划任务。调度任务通过同一 `/usr/local/bin/litradar` 启动短生命周期的 `index`、`notify` 或 `push` 子进程；这些子进程不是 Compose 服务。

SIGINT/SIGTERM 会协调关闭 HTTP 与调度组件。若任务子进程正在运行，应用会终止并等待它，把运行状态保存为 `cancelled`，且不再启动该任务的剩余步骤。HTTP、心跳或调度组件意外失败会关闭整个进程并返回非零状态。

## 镜像内容

根 Dockerfile 包含以下构建阶段；Dockerfile frontend、Node、Rust 和 Debian 引用都同时保留可读 tag 与不可变 digest：

1. Node.js 24 Alpine 只复制 `app/package.json` 和 lockfile，使用缓存安装依赖。
2. 独立前端构建阶段复制 `app/` 源码，生成 `out/`，并为 HTML、CSS、JavaScript、JSON、SVG、TXT、XML 和 source map 保留原文件及确定性 gzip 兄弟文件。
3. `rust:1.96-bookworm` 只构建 release `litradar` 目标；workspace release profile 执行 symbol stripping，并用 BuildKit cache mount 复用 Cargo registry、git 与 target 产物。
4. `debian:trixie-slim` 只复制 `/usr/local/bin/litradar`、不可变 Meta bundle 到 `/usr/share/litradar/meta`，以及静态站点到 `/app/web`。

运行层安装 CA 证书、`curl` 和非 root 账户所需的最小系统包，随后切换到固定 UID/GID `10001:10001`。当前内容 schema v6 使用 SQLite 内建 `unicode61`，镜像不复制或加载历史 `simple` 原生扩展。最终镜像不包含其他 LitRadar 可执行文件、Node.js、Next.js standalone、`server.js` 或 Python 运行时。镜像自身定义 readiness `HEALTHCHECK` 和 `SIGTERM` stop signal。默认 `ENTRYPOINT` 与 `CMD` 已包含应用、`serve` 子命令和密钥路径，因此本地 Compose 不覆盖命令；自行使用 `docker run` 时仍必须把 32 字节密钥只读挂载到该路径。

release profile 没有设置 LTO、codegen unit 或 `panic = "abort"`，保留 Cargo 默认 codegen/链接并使用 unwind，使现有任务监管和清理路径不因发布优化而改变。T22 实测的 Thin LTO + 单 codegen unit 冷容器构建在 30 分钟硬上限内没有产出镜像，因此被拒绝；不能仅凭理论体积收益接受不可执行的构建成本，也不能牺牲进程树、任务取消或容器 smoke 门禁。当前只保留 symbol stripping，最终时长和体积证据记录在同任务验证结果中。

T22 在 Windows Docker Desktop 29.6.1 的本地冷构建中，最终策略总耗时 218.5 秒，其中 Rust release 208.7 秒；stripped 可执行文件为 35,217,016 字节，无预置 provenance 的 smoke 镜像为 83,901,821 字节。全部层命中缓存后，同一 manifest 的重建为 6.3 秒。硬件与远端缓存会改变时长，这些数值用于说明已接受的策略成本，不是跨机器性能承诺。

支持 gzip 的客户端会直接收到预压缩文件，不支持的客户端仍收到原文件。`/_next/static/*` 成功响应使用长期 public immutable 缓存；页面、导航 payload 和导出的 404 使用 `no-cache`；认证/API 的私有缓存边界不因此放宽。

## 本机或 loopback 首次部署

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

这里的 `latest` 只用于本机初始配置和验证，不是生产部署输入。生产升级不得执行只使用 `docker-compose.yml` 的上述命令。

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
- 每个 CSV/内容库的索引 Provider，以及摘要页/全文的默认和 catalog 覆盖顺序
- 跨源 CORS
- MCP Host/Origin
- Secure Cookie
- 日志格式和严格 filter

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

### 6. 中断恢复和更新

每条实时索引或更新命令先在 `data/index-control/index-batches.sqlite` 的 batch ledger schema v2 中取得固定单行项目租约 `index_batch_lease`，因此同一项目跨全部目录只允许一个 active invocation。进入某个目录后，命令还会在该目录的 catalog 控制库 v4 中取得 `(catalog_name, provider_name)` 的 `provider_leases` 次级租约；它保护 Provider traversal 和内容提交，但不替代项目级串行化。父进程每 30 秒把两级租约续到未来 300 秒。普通上游、worker 或清单错误保留 active batch、catalog phase 和待发布事件并释放当前所有权；容器或 Docker daemon 被强制终止时，后续命令只能在确认旧进程消失且租约过期后接管兼容 batch。

恢复时按以下顺序操作：

1. 确认旧容器、计划任务子进程和 `litradar-memory-*` 画像容器已经停止；不要通过删除 `index_batch_lease` 或 catalog `provider_leases` 绕过所有权检查。
2. 停止常驻服务并完成离线、已验证的当前数据备份。部署密钥必须继续留在 Compose secret 中，不得复制到备份或日志。
3. 普通失败可立即重跑同一命令；硬终止必须等到旧租约过期。未过期时的明确所有者错误表示旧运行仍受保护，不是可忽略的重试提示。
4. 需要恢复 changes JSON 时必须用兼容的目录选择、sync mode、ledger 中保存的遗留 issue-batch 恢复值和 notify flags 重跑 `--update`，让默认 resume 继续同一 active batch。issue-batch 只用于匹配旧 active batch；若非默认值要求显式传入，CLI 会发出兼容性警告。存在已发布或可能已发布 manifest 时不要用 `--no-resume` 丢弃 handoff。
5. 成功后确认命令退出 0、changes JSON 可解析、batch 历史进入 `completed`、`index_batch_lease` 与 catalog `provider_leases` 都没有活动所有者，再启动服务并检查 `/health/live`、`/health/ready` 和 `/`。

Scholarly 增量使用成功期次 anchor 年份的 1 月 1 日作为日期下界，并完整补查 candidate 到 base 的边界期次；不是从完成时间回看 30 天。Crossref 保留 `from-update-date`，冻结 UTC 秒上界 `T`，按完整 created 历史分片；小片以最多 1000 条单响应校验，单秒仍过大才使用无排序 cursor，完整计数通过后在本地归并期次。OpenAlex 保留原有 `from_created_date` 和有序分页。缺少可用 anchor 或无法证明边界时进行同源无界重放，已有内容不因空结果被删除。规则和非快照限制见 [Scholarly](../reference/sources/scholarly.md)。

同次 Crossref resume 不再使用 240 秒游标过期或 HTTP 500 强制重扫；下一次 update 仍须新查询并保留整期补查。升级后的 English 恢复使用原正确性选项和默认 `--resume`，只选 `english_journals.csv`，本次不发送通知；完整示例见 [CLI 恢复步骤](../reference/cli.md#crossref-2026-08-游标升级后的-english-恢复)。这次 API 兼容升级不要求删除内容库、控制库或重建约 10 GB 的 English 数据；新 traversal v2 不能由旧二进制直接恢复。

CNKI 的 2xx 正文解码失败会在现有三次上限内记录并重试；持续失败仍应作为上游/工作流失败处理，不能因为当时内存较低就算作验收通过。

当前 v6 备份与恢复验证不依赖平台原生 tokenizer。若历史快照的 `sqlite_schema` 仍声明 `tokenize='simple'`，它不是可直接服务的当前 v6 数据库；必须先走受支持的迁移或重建并完成完整性、外键、schema 和投影计数检查，不能通过向生产镜像临时复制 DLL/SO 绕过版本边界。

## 数据和秘密

| 宿主机或镜像路径         | 容器路径                    | 说明                          |
| ------------------------ | --------------------------- | ----------------------------- |
| `./data`                 | `/app/data`                 | 唯一持久可写业务挂载          |
| `./secrets/litradar.key` | `/run/secrets/litradar_key` | Compose secret，只读          |
| 镜像官方 Meta bundle     | `/usr/share/litradar/meta`  | 不可变源；不在 `/app/data` 内 |

Crossref 的可丢弃工作集固定在 `/app/data/index-work/scholarly/`，随现有 data 挂载持久化，不增加挂载或公开配置，也不进入备份。它不使用 `/tmp` 大排序文件，现有 64 MiB tmpfs 保持不变。每个工作集的 SQLite page cache 为 4 MiB、mmap 关闭、主文件上限 4 GiB；事务日志需要额外可用空间，4 MiB 不是进程 RSS 上限，HTTP JSON 仍受 16 MiB 限制。

容量不足会明确失败并保留正式内容和旧 anchor，不能通过截断工作集宣告完成。未完成的自有缓存通常供 resume 复用；缺失或可识别损坏时，当前版本从同一冻结范围重新收集。路径、归属或 symlink/reparse point 校验失败则拒绝访问。不要在运行中手工清空工作目录，也不要用删除控制状态代替容量诊断。

### Meta bundle 与持久卷

Docker bind mount 和 Kubernetes PVC 会遮蔽挂载点中的镜像层内容，不会执行目录合并。LitRadar 因此把官方源与持久副本分开：Dockerfile 固定把 bundle 复制到 `/usr/share/litradar/meta`，应用仅在精确的 `/usr/share/litradar/meta/bundle-manifest.json` 存在时，于数据库迁移后把清单允许的更新同步到 `/app/data/meta`。该位置不是环境变量、CLI 或管理员可覆盖项，也不能改指向可写的持久目录。

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

最终容器不声明 LitRadar 应用专用环境配置。浏览器 API 同源、开发代理、Meta bundle、日志和父子进程关联分别由确定性代码路径、数据库运行设置或隐藏内部参数负责；部署仍通过 CLI 参数和只读密钥文件提供监听、路径与秘密边界。

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
- `mem_limit: 160m`
- `cap_drop: [ALL]`
- `no-new-privileges:true`
- 带 `noexec,nosuid,nodev` 的 `/tmp` tmpfs
- 唯一持久可写的数据挂载、只读密钥挂载和镜像内 readiness 健康检查

除 `/app/data` 外没有持久写路径。`/app/web` 随镜像只读提供，运行时不生成 Next.js cache。不要通过 root 容器、开放整个宿主机目录或挂载 Docker socket 解决权限问题。

`160m` 由 Compose 渲染为 167,772,160 字节的 cgroup v2 `memory.max`。该限制同时适用于 `docker compose up` 和 `docker compose run`；内嵌调度启动的子进程与 `serve` 共享同一个限制。它是高于 120 MiB 作业峰值门禁的失控保护，不是可用内存目标。触发硬上限可能直接终止作业，因此不能用它替代画像和低内存默认值。自行使用 `docker run` 时必须显式提供等价的 `--memory 160m`。

上游 source 响应在透明解压后使用 endpoint 级字节上限：CNKI/ZJLib 2 MiB、JFBYM 256 KiB、Scholarly 16 MiB，PDF 另有 32 MiB 上限。Content-Length 只是前置快速拒绝，实际还会对解压后流执行 `limit + 1` 读取；这些应用内上限是 160 MiB cgroup 之前的必要资源边界，不能被容器 OOM 代替。

## 日志收集与轮转

应用默认把 JSON Lines 写入 `stderr`，不在 `/app` 创建日志文件。Compose 使用 Docker `local` 驱动并设置 `max-size=10m`、`max-file=5`、`compress=true`；每个容器在压缩影响之前最多约 50 MiB 驱动日志，同时保持 `read_only: true` 和唯一数据写卷不变。

```bash
docker compose logs --since 30m --timestamps litradar
docker compose logs --no-log-prefix litradar | jq -c 'select(.level == "ERROR")'
```

删除容器会连同其驱动日志一起删除；需要跨轮转或跨容器保留的事故证据必须在窗口内导出。不要直接读取 Docker 内部 driver 文件。事件 schema、request/run ID 关联、丢失语义、浏览器本地范围和事故流程见[日志运维](logging.md)。

## 内存画像与门禁

仓库提供 PowerShell 7 脚本 `scripts/profile_docker_memory.ps1`。脚本为每次运行生成唯一的 Compose 项目、容器和网络，只删除这些具名测试资源，并把不含命令参数和密钥值的 JSON 写入已忽略的 `output/memory/`。Docker 必须使用 cgroup v2，目标镜像必须先构建：

```powershell
docker compose build litradar
docker compose config --quiet
```

`-DataPath` 是强制参数。脚本会把该目录以读写方式挂载到 `/app/data`，服务启动会迁移其中的数据库，索引/更新会写入其中的数据。确定性测试应传隔离副本；只有完成停机和已验证备份后，才可把真实 `./data` 传给更新场景。部署密钥仍由 Compose secret 挂载，命令只传容器内密钥路径。

### 指标口径

| JSON 字段/来源                   | 含义                                                                         | 用途                               |
| -------------------------------- | ---------------------------------------------------------------------------- | ---------------------------------- |
| `Memory.WorkingSet*`             | `memory.current - memory.stat.inactive_file`，与 Docker working-set 口径一致 | 20/24 和 100/120 MiB 门禁          |
| `Memory.CgroupCurrent*`          | 原始 cgroup 当前用量，包含可回收文件页缓存                                   | 分析页缓存和 cgroup 总占用         |
| `Memory.CgroupLifetimePeakBytes` | 容器创建以来的原始 `memory.peak`                                             | 诊断启动或作业瞬时峰值             |
| `PeakProcesses`                  | `docker top` 的进程 RSS、线程数和命令名峰值拆分                              | 区分 `serve`、作业子进程和辅助进程 |
| `Memory.SwapPeakBytes`           | `memory.swap.current` 的采样峰值                                             | 必须为 0                           |
| `EventDelta`                     | 新建 cgroup 生命周期内的 `memory.events` 计数                                | `max`、`oom`、`oom_kill` 必须为 0  |
| `FullPressureAvg10Max`           | `memory.pressure` 的 full `avg10` 采样最大值                                 | 验收窗口应为 0                     |

每个样本还保存选定的 `memory.stat`、PSI、进程 RSS 总和、进程数和线程数。摘要报告 working-set 的 p50、p95、采样峰值、持续时间、场景退出码、OOM 状态和门禁失败原因。采样需要短生命周期的 `docker exec`，因此 cgroup 数值是略偏保守的；进程 RSS 与 cgroup working set 的记账方式不同，不能相加。

默认门禁：

| 场景                                 | p95     | 采样峰值 |
| ------------------------------------ | ------- | -------- |
| `warm-idle`                          | 20 MiB  | 24 MiB   |
| `index`、`update`、`scheduled-child` | 100 MiB | 120 MiB  |

所有场景还要求 swap、OOM 和 `memory.events.max` 为 0、业务命令退出 0。`-P95LimitMiB` 和 `-PeakLimitMiB` 可显式覆盖阈值；这用于独立预算或门禁自测，不改变生产目标。`-ExpectedMemoryLimitMiB 160` 会同时校验实际容器限制。任何并发覆盖，尤其是提高 `--processes` 或 `--workers`，都必须使用相同数据和场景重新画像；遗留 `--issue-batch` 不改变当前运行时并发或内存，因此不是画像调优参数。

### 场景命令

真实数据只在停机备份验证完成后画像。最终验收顺序为默认恢复索引、CCF 更新、中国期刊更新、英文学术更新、同 cgroup 计划任务子进程，最后是预热后的日常服务。`DurationSeconds` 是保护性超时；作业提前完成时立即结束，超时则以退出码 124 失败。

默认恢复索引不带 `--update`，用于证明已完成期刊可跳过且旧待发布事件不会被非更新运行接管：

```powershell
pwsh ./scripts/profile_docker_memory.ps1 `
  -Scenario index `
  -DataPath ./data `
  -DurationSeconds 14400 `
  -Command @(
    'index',
    '--secret-key-file', '/run/secrets/litradar_key'
  ) `
  -ExpectedMemoryLimitMiB 160 `
  -OutputPath ./output/memory/final-resume-index.json
```

先运行需要恢复待发布事件的 CCF 更新，再分别运行中国期刊和英文学术更新：

```powershell
pwsh ./scripts/profile_docker_memory.ps1 `
  -Scenario update `
  -DataPath ./data `
  -DurationSeconds 14400 `
  -Command @(
    'index',
    '--secret-key-file', '/run/secrets/litradar_key',
    '--file', 'ccf_computer_journals.csv',
    '--update'
  ) `
  -ExpectedMemoryLimitMiB 160 `
  -OutputPath ./output/memory/final-update-ccf.json

pwsh ./scripts/profile_docker_memory.ps1 `
  -Scenario update `
  -DataPath ./data `
  -DurationSeconds 14400 `
  -Command @(
    'index',
    '--secret-key-file', '/run/secrets/litradar_key',
    '--file', 'chinese_journals.csv',
    '--update'
  ) `
  -ExpectedMemoryLimitMiB 160 `
  -OutputPath ./output/memory/final-update-chinese.json

pwsh ./scripts/profile_docker_memory.ps1 `
  -Scenario update `
  -DataPath ./data `
  -DurationSeconds 14400 `
  -Command @(
    'index',
    '--secret-key-file', '/run/secrets/litradar_key',
    '--file', 'english_journals.csv',
    '--update'
  ) `
  -ExpectedMemoryLimitMiB 160 `
  -OutputPath ./output/memory/final-update-english.json
```

常驻服务和同 cgroup 子任务的合并画像：

```powershell
pwsh ./scripts/profile_docker_memory.ps1 `
  -Scenario scheduled-child `
  -DataPath ./data `
  -DurationSeconds 14400 `
  -Command @(
    'index',
    '--secret-key-file', '/run/secrets/litradar_key',
    '--file', 'ccf_computer_journals.csv',
    '--update'
  ) `
  -ExpectedMemoryLimitMiB 160 `
  -OutputPath ./output/memory/final-scheduled-child.json
```

五分钟预热后采集十分钟日常服务和轻流量；路径必须包含 `/health/live`、`/health/ready` 和 `/`：

```powershell
pwsh ./scripts/profile_docker_memory.ps1 `
  -Scenario warm-idle `
  -DataPath ./data `
  -WarmupSeconds 300 `
  -DurationSeconds 600 `
  -TrafficPath /health/live,/health/ready,/ `
  -ExpectedMemoryLimitMiB 160 `
  -OutputPath ./output/memory/final-warm-idle.json
```

每份 JSON 只记录 `CommandProvided`，不保存命令参数或密钥值。验收要求 `Gate.Passed=true`，并同时满足：作业 working-set p95 不超过 100 MiB、采样峰值不超过 120 MiB；日常服务分别不超过 20 MiB 和 24 MiB；退出码为 0；swap、OOM、`memory.events.max` 增量和 full PSI `avg10` 都为 0。任一条件失败都不能用其他较低指标抵消。

如果 `Gate.Failures` 只有非零/124 退出码或上游错误，这是来源或工作流失败，当前样本不能作为内存验收；先处理来源问题再完整重跑。如果命令退出 0 但 p95、峰值、swap、OOM、max event 或 PSI 失败，这是内存门禁失败。脚本对两类情况都返回 1。可用极小的显式阈值验证门禁确实失败：

```powershell
pwsh ./scripts/profile_docker_memory.ps1 `
  -Scenario warm-idle `
  -DataPath ./isolated-profile-data `
  -DurationSeconds 10 `
  -P95LimitMiB 0.001 `
  -PeakLimitMiB 0.001 `
  -ExpectedMemoryLimitMiB 160
```

该命令预期返回 1，并在 JSON 的 `Gate.Failures` 中同时列出 p95 和峰值超限。中断或失败时 `finally` 仍只按本次唯一名称删除容器和网络；若宿主机或 Docker daemon 被强制终止，可用 `docker ps -a --filter name=litradar-memory-` 检查后按完整名称清理。

## 公网部署

### 1. 取得并验证发布 digest

`Build and Push Docker Image` 工作流只构建并推送一次无 tag 候选。它从 Buildx 捕获 registry digest，重新拉取 `repository@sha256:...` 运行 hardened smoke，随后才为同一 digest 生成 SPDX SBOM、SLSA provenance、GitHub attestation 和 Cosign keyless signature。最后一步在不重建的前提下创建 `sha-<40-character commit>` tag，并把 `latest` 更新为同一个已验证 digest。`latest` 只用于本地便利和版本发现；生产部署仍必须从成功的 workflow summary 取得并验证精确 digest。

从成功 workflow summary 复制 64 位 digest，不要从 tag 推测：

```bash
export LITRADAR_IMAGE_REPOSITORY=ghcr.io/qianfuv/litradar
export LITRADAR_IMAGE_DIGEST='<64 lowercase hexadecimal characters>'
[[ "$LITRADAR_IMAGE_DIGEST" =~ ^[0-9a-f]{64}$ ]]
export LITRADAR_IMAGE_REFERENCE="${LITRADAR_IMAGE_REPOSITORY}@sha256:${LITRADAR_IMAGE_DIGEST}"
```

使用发布 workflow 的精确身份验证签名，并分别验证 provenance 与 SPDX SBOM attestation：

```bash
cosign verify \
  --certificate-identity 'https://github.com/QianFuv/LitRadar/.github/workflows/docker.yaml@refs/heads/main' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  "$LITRADAR_IMAGE_REFERENCE"

gh attestation verify "oci://${LITRADAR_IMAGE_REFERENCE}" \
  --repo QianFuv/LitRadar \
  --predicate-type https://slsa.dev/provenance/v1

gh attestation verify "oci://${LITRADAR_IMAGE_REFERENCE}" \
  --repo QianFuv/LitRadar \
  --predicate-type https://spdx.dev/Document/v2.3
```

任一验证失败、workflow 的源码 commit 不匹配、或 registry digest 与 summary 不一致时停止部署。不得把证书 identity 改成通配符，也不得退回 tag 运行。

### 2. 预置生产安全设置

先通过只绑定 loopback 的本地配置完成管理员 bootstrap，在管理员运行配置中设置 `secure_cookies=true`，并配置准确的 CORS Origin、MCP Host/Origin、trusted proxy 与认证限流。随后停止本地服务：

```bash
docker compose down
```

生产覆盖文件固定加入 `--require-secure-cookies`；数据库值仍为 false 时，服务会在绑定端口前失败，不能降级启动。

### 3. 验证并启动生产 Compose

`compose.production.yaml` 使用 `!reset` 删除本地 build 和端口发布，并把镜像强制组装为 `${LITRADAR_IMAGE_REPOSITORY}@sha256:${LITRADAR_IMAGE_DIGEST}`。需要 Docker Compose 2.24.4 或更新版本。先检查解析结果：

```bash
docker compose \
  -f docker-compose.yml \
  -f compose.production.yaml \
  config --images

docker compose \
  -f docker-compose.yml \
  -f compose.production.yaml \
  config --format json > /tmp/litradar-production-compose.json
```

`config --images` 必须只输出上面已经验证的 digest reference；JSON 中不得存在 service `build` 或 `ports`，command 必须包含 `--require-secure-cookies`，`/tmp` 必须包含 `noexec,nosuid,nodev`。确认后启动：

```bash
docker compose \
  -f docker-compose.yml \
  -f compose.production.yaml \
  up -d --pull always --remove-orphans
```

### 4. 完成基础设施边界

生产环境必须在同一容器网络加入只发布 HTTPS `443` 的反向代理，并把 Web、API、Swagger/OpenAPI 和 MCP 全部路径转发到 `litradar:8000`。不要用 `0.0.0.0` 宿主机端口替代反向代理。

应用 URL 校验不能替代网络策略。容器或主机 egress ACL 只应允许管理员批准的 AI/PushPlus 目标、DNS 和 TLS 基础设施，并显式阻断 loopback、RFC1918、link-local、元数据地址与内部服务网段。多实例部署还必须在反向代理/网关使用共享认证限流；应用内单实例 bucket 不是跨实例协调器。

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

1. 运行 `docker compose logs --since 30m litradar`，按 `index.*`、`source.*` 和 `error_kind` 查询。
2. 确认宿主机 `data/index/*.sqlite` 存在。
3. 确认 bind mount 权限。
4. 按 CLI 参考运行单个 CSV 索引。

### readiness 返回 503

1. 运行 `docker compose logs --since 30m litradar`，查找 `scheduler.tick.*`、`service.component.failed` 或安全错误分类。
2. 确认应用可以写入同一个 `data/auth.sqlite`。
3. 确认没有长时间阻塞的任务；调度心跳健康窗口为 90 秒。
4. 用 `docker compose run --rm litradar scheduler validate --secret-key-file /run/secrets/litradar_key` 验证已保存任务。

### 历史 `simple` tokenizer 索引

当前内容 schema v6 的 `article_search` 固定使用内建 `unicode61`，生产镜像故意不包含 `libsimple.so`，查询连接也不会按固定路径加载扩展。若恢复数据仍声明 `tokenize='simple'`，先停止服务并按数据库版本执行受支持的迁移或重建；迁移后必须复核 schema 版本和 FTS 定义，再运行只读完整性与查询检查。不要把复制历史扩展进容器当作修复，固定路径上偶然存在或损坏的原生文件不得影响合法 v6 查询。

### 通知没有结果

检查 `.changes.json`、用户偏好、AI/PushPlus 凭据，以及认证库中的 `delivery_runs`、`delivery_run_items`、`delivery_dedupe` 和 `delivery_leases`。不要通过修改旧状态 JSON 来清除 busy/unknown；详见[通知与追踪](../guides/notifications.md)。
