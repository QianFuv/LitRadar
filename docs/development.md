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

Docker 运行时由 `api` 提供 API，由 `ps-cli worker shadow` 作为 sidecar 加载定时任务配置。`ps-cli scheduler` 和 `ps-cli worker` 是调度内部入口；用户侧保留 `api`、`index`、`notify` 和 `push` 四个旧命令名。

## Rust 模块划分

| Crate | 职责 |
| --- | --- |
| `ps-api` | Axum API 服务，保持现有 `/api/*` 契约 |
| `ps-cli` | scheduler、worker、测试辅助和共享 CLI 调度入口 |
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

### Rust worker

```bash
cargo run -p ps-cli -- worker shadow --interval-seconds 300
```

### Scheduler

```bash
cargo run -p ps-cli -- scheduler dry-run
cargo run -p ps-cli -- scheduler run-once 1
cargo run -p ps-cli -- scheduler dry-run-once 1
```

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

- `OPENALEX_API_KEY_POOL`
- `SEMANTIC_SCHOLAR_API_KEY_POOL`
- `CROSSREF_MAILTO_POOL`
- `PROXY_POOL`

Rust API、worker 和 CLI 启动时会应用这些配置；数据库已有值会覆盖同名进程环境变量。中文全文凭证保存在用户级 CNKI session 表中，测试时可继续复用 `data/auth.sqlite` 中的有效凭证；凭证失效时应使用离线 fixture 测试。

## 修改代码后的检查

Rust 后端改动：

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Rust 后端覆盖率摘要：

```bash
cargo llvm-cov --workspace --summary-only
```

覆盖率测试应优先补确定性业务行为。不要通过排除生产模块来凑数；薄二进制入口、进程全局 tracing 初始化、OS signal、真实网络适配器和无限循环 worker 这类剩余缺口应在任务记录中说明。

前端改动：

```bash
cd app
pnpm exec tsc --noEmit
```

部署相关改动：

```bash
docker compose build
```

## 常见误区

### 每周更新不是按文章日期直接扫描

每周更新、通知和追踪推送都依赖 `data/push_state/*.changes.json` 或状态快照差异。没有变更清单时，相关链路可能为空。

### 管理员定时任务不是 API 进程内 APScheduler

Rust worker sidecar 会加载任务配置；单次执行由 `ps-cli scheduler run-once TASK_ID` 触发，dry-run 由 `ps-cli scheduler dry-run-once TASK_ID` 触发。

### 通知配置不绑定单一 AI 服务商

通知链路使用 OpenAI 兼容接口。全局默认值来自 `NOTIFY_AI_*` 环境变量，用户级配置优先。
