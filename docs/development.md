# 开发指南

本文档说明当前 Rust 后端架构、数据流、运行命令和开发检查。旧 Python 后端和迁移契约测试已经移除，正常开发、部署和调度都使用 Rust 命令。

## 整体架构

```text
CSV / existing index data
  -> index
  -> data/index/*.sqlite
  -> api
  -> Next.js app

data/push_state/*.changes.json
  -> notify
  -> push
  -> /api/tracking/push-weekly
```

Docker 运行时由 `api` 提供 API，由 `worker` 作为 sidecar 按 cron 执行启用的定时任务。用户侧命令为 `api`、`index`、`notify`、`push`、`scheduler` 和 `worker`。

## 启动与数据库迁移

所有正式入口都以版本化 SQLite migration 作为业务访问的前置条件：

1. 先完成参数和配置路径解析
2. 迁移 `data/auth.sqlite` 以及本次入口会使用的 `data/index/*.sqlite`
3. API 迁移成功后才绑定监听端口；worker 迁移成功后才进入循环
4. 迁移失败或发现高于当前二进制支持的 `PRAGMA user_version` 时立即退出

迁移实现位于 `crates/ps-storage/src/migrations.rs`。新增 schema 变更时必须追加下一个有序版本，在独立事务中修改 schema 并在同一事务内更新 `user_version`；不要在 repository 查询函数或连接 helper 中加入 `CREATE TABLE`、`ALTER TABLE` 或迁移判断。迁移至少要覆盖空库、代表性旧库、当前库幂等、失败回滚和未来版本拒绝测试。

## Rust 模块划分

| Crate | 职责 |
| --- | --- |
| `ps-api` | Axum API 服务，保持现有 `/api/*` 契约 |
| CLI support crate | 独立命令的共享解析和调度库 |
| `ps-auth` | 认证、密码、令牌和 Cookie 兼容逻辑 |
| `ps-storage` | SQLite auth/index 存储访问 |
| `ps-index` | 索引 schema、写库、live 索引和变更清单 |
| `ps-sources` | Crossref/OpenAlex/Semantic Scholar/CNKI source 客户端 |
| `ps-recommend` | 通知候选、AI 选择、PushPlus 内容和状态文件逻辑 |
| `ps-worker` | scheduler 加载和通知/追踪分发编排 |
| `ps-domain` | 共享领域结构 |

## 真实数据流

### 索引流

1. `index` 读取 `data/meta/*.csv`
2. `ps-sources` 调用 Crossref、OpenAlex、Semantic Scholar 或 CNKI overseas
3. `ps-index` 写入 `journals`、`issues`、`articles`、`article_listing`、`article_search`
4. `--update` 输出 `data/push_state/*.changes.json` 变更清单

现有生产索引库可以直接放在 `data/index/` 下供 Rust API 读取。

### 检索流

1. 前端通过 `app/lib/api.ts` 请求 `/api/*`
2. Rust API 按 `db` 参数解析 `data/index/*.sqlite`
3. `/api/articles` 优先使用 `article_listing`
4. 全文检索联动 `article_search`
5. 返回与旧 API 兼容的分页响应

### 通知与追踪流

1. 读取 `data/push_state/*.changes.json` 或状态快照差异
2. 根据 `notification_settings` 加载订阅用户
3. 使用 OpenAI 兼容配置做候选筛选
4. `notify` 生成 PushPlus 发送计划
5. `push` 生成追踪文件夹写入计划

`--dry-run` 不执行外部发送或收藏写入副作用。

## 本地运行

### Rust API

```bash
cargo run --bin api
```

默认后端地址：`http://127.0.0.1:8000`。

启动后可访问：

- `http://127.0.0.1:8000/docs/`：Swagger UI 交互式 API 文档
- `http://127.0.0.1:8000/openapi.json`：由 Rust route 注解和 DTO schema 编译期生成的 OpenAPI JSON

API 默认输出 HTTP 请求日志，包含 method、path、status 和 latency。设置 `RUST_LOG` 可覆盖默认过滤器：

```bash
RUST_LOG=error cargo run --bin api
RUST_LOG=ps_api=debug,tower_http=debug cargo run --bin api
```

### 本机管理员初始化

空用户库不会接受公开首用户注册。开发环境通过 stdin 创建一次管理员：

```bash
printf '%s\n' "$ADMIN_PASSWORD" | cargo run --bin admin -- bootstrap --username admin --password-stdin
```

不要添加接受 `--password VALUE` 的参数，也不要在调试日志中输出 stdin 内容。bootstrap 只在用户表为空时成功；测试应使用临时数据库，并覆盖并发调用只有一个成功的情况。

登录与注册限流保存在单个 `ApiState` 的有界内存结构中。用户名键会转为小写并定期清理，登录和注册另有各自的全局固定窗口。时间相关测试直接向 limiter 传入确定性秒值，不使用 sleep。

### Rust worker

```bash
cargo run --bin worker -- --interval-seconds 30
```

`worker` 使用 `scheduler_state.last_checked_at` 记录已检查到的墙钟时间，而不是只判断当前分钟。每轮会回看上次游标到当前时间的 UTC 分钟，最多回看 24 小时，再转换到任务的 IANA 时区匹配五段 cron。夏令时前跳缺失的本地分钟不会生成运行，回拨重复的本地分钟会生成两个不同 UTC 槽。任务默认 `coalesce = true`，因此离线后只补跑最近一个错过的槽；关闭后会保留回看窗口内的每个槽。

`scheduled_task_runs` 通过 `(task_id, scheduled_for)` 唯一约束和 `BEGIN IMMEDIATE` 认领事务协调多个 worker。每个任务同时最多有一个认领或运行实例。认领后尚未开始且租约过期的运行可以重新认领；已开始但心跳过期的运行被标为 `unknown`，不会自动重复执行。worker 与运行租约会在长任务期间续期，管理员可通过 `/api/admin/scheduler/status` 查看按 90 秒窗口计算的健康状态。

任务必须是 `index`、`notify` 或 `push` 类型化 job。worker 根据白名单字段构造 argv，直接启动对应可执行文件，不经过 shell。`timeout_seconds` 限制完整 job 链，stdout/stderr 只以有界摘要保存在数据库内部，不通过管理 API 返回。索引 job 可串联 notify/push，任一步失败都会停止该任务的后续步骤；不同任务并发执行，一个任务失败或超时不会阻止同轮其他任务。

### Scheduler

```bash
cargo run --bin scheduler -- validate
cargo run --bin scheduler -- run-once 1
cargo run --bin scheduler -- dry-run-once 1
```

从旧 `command` schema 迁移的记录只保留为禁用的审阅数据。`run-once` 也不会执行这类记录；管理员必须在后台选择结构化预设并保存，才能生成新的 job。API 与存储层都校验 cron、文件名和参数范围，worker 在执行前再次校验。

### 索引

```bash
cargo run --bin index -- --file english_journals.csv --update
cargo run --bin index -- --file cnki_journals.csv --resume --issue-batch 10
```

省略 `--file` 时会处理 `data/meta/` 下所有 CSV。`--processes` 控制单个 CSV 内的期刊 worker 进程数，`--workers` 控制每个 worker 内的 CNKI 文章详情请求并发数，`--issue-batch` 控制 CNKI 每轮合并的 issue 数。`--update` 会生成 `data/push_state/*.changes.json`，`--notify --notify-dry-run` 可在更新后串联通知 dry-run。

### 通知与追踪

```bash
cargo run --bin notify -- --dry-run
cargo run --bin notify -- --db utd24.sqlite --changes-file data/push_state/utd24.changes.json --no-dry-run
cargo run --bin push -- --dry-run
cargo run --bin push -- --db utd24.sqlite --changes-file data/push_state/utd24.changes.json --no-dry-run
```

省略 `--db` 时会处理 `data/index/*.sqlite`。`notify` 默认状态目录是 `data/push_state`，`push` 默认状态目录是 `data/folder_push_state`。

### 前端

```bash
cd app
corepack enable pnpm
pnpm install
pnpm dev
```

前后端分离运行时，可通过 `NEXT_PUBLIC_API_URL` 指定 API 根地址。

## 运行时配置

外部元数据服务配置由管理员后台 `/api/admin/runtime-settings` 写入 `data/auth.sqlite` 的 `runtime_settings` 表。当前受管理配置包括：

- `openalex_api_key_pool`
- `semantic_scholar_api_key_pool`
- `crossref_mailto_pool`
- `cors_allowed_origins`
- `mcp_allowed_hosts`
- `mcp_allowed_origins`
- `secure_cookies`

Rust API、worker 和 CLI 启动时会读取这些数据库配置，不读取进程环境变量作为运行配置。中文全文凭证保存在用户级 CNKI session 表中，测试时可继续复用 `data/auth.sqlite` 中的有效凭证；凭证失效时应使用离线 fixture 测试。

## 修改代码后的检查

Rust 后端改动：

```bash
cargo fmt --all -- --check
cargo sort --workspace --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

Rust 后端覆盖率摘要：

```bash
cargo llvm-cov --workspace --summary-only
```

覆盖率测试应优先补确定性业务行为。不要通过排除生产模块来凑数；薄二进制入口、进程全局 tracing 初始化、OS signal、真实网络适配器和无限循环 worker 这类剩余缺口应在任务记录中说明。

前端改动：

```bash
cd app
pnpm generate:api:check
pnpm lint
pnpm format:check
pnpm exec tsc --noEmit
pnpm test
pnpm test:e2e
pnpm build
```

`pnpm generate:api` 先运行 Rust `openapi` emitter，再用 `openapi-typescript` 生成已签入的 JSON/TypeScript 契约。认证、定时任务、后台推送状态和包含凭证字段的设置响应还会经过 `app/lib/api-contract.tsx` 的运行时校验；不要把这些调用退回为仅靠泛型断言的 JSON 解析。

Vitest 使用 jsdom、Testing Library 和 MSW，覆盖认证恢复、查询序列化、游标分页、收藏缓存更新、追踪轮询与管理员 mutation。Playwright 只使用 `page.route` 本地 fixture，不依赖真实后端或上游服务；首次本地运行需要执行 `pnpm exec playwright install chromium`。

部署相关改动：

```bash
docker compose config --quiet
docker compose build
```

运行时镜像必须保持非 root。根 Compose 还要求只读根文件系统、可写 `/tmp` tmpfs、显式可写 `data` 挂载、`cap_drop: [ALL]`、`no-new-privileges`、服务健康检查与 `restart: unless-stopped`。Linux 原生 Docker Engine 的 fixture 数据目录应归固定后端 UID/GID `10001:10001`；不要通过恢复 root 用户来绕过权限失败。

## 常见误区

### 每周更新不是按文章日期直接扫描

每周更新、通知和追踪推送都依赖 `data/push_state/*.changes.json` 或状态快照差异。没有变更清单时，相关链路可能为空。

### 管理员定时任务不是 API 进程内 APScheduler，也不是 shell 命令

Rust worker sidecar 会按持久化游标、时区和 cron 自动执行启用的类型化任务；单次执行由 `scheduler run-once TASK_ID` 触发，dry-run 由 `scheduler dry-run-once TASK_ID` 触发。不要重新加入自由命令字段或命令解释器；新增调度能力时应同步扩展领域枚举、参数校验、运行认领、OpenAPI 与管理 UI。

### 通知配置不绑定单一 AI 服务商

通知链路使用 OpenAI 兼容接口。AI 与 PushPlus 凭据来自用户级通知设置；未配置可用 AI key/model 的用户会被跳过。
