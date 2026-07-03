# 开发指南

本文档说明当前 Rust 后端架构、数据流、运行命令和开发检查。Python 后端模块仍保留在仓库中，但只作为契约测试、fixture 比对和历史兼容参考，不再作为正常后端运行入口。

## 整体架构

```text
CSV / fixture / existing index data
  -> ps-cli index fixture
  -> data/index/*.sqlite
  -> ps-api
  -> Next.js app

data/push_state/*.changes.json
  -> ps-cli notify dry-run|shadow
  -> ps-cli push dry-run|shadow
  -> /api/tracking/push-weekly
```

Docker 运行时由 `ps-api` 提供 API，由 `ps-cli worker shadow` 作为 sidecar 加载定时任务配置。Python 目录用于测试 Rust 兼容性，不再通过 package scripts 暴露 `api`、`index`、`notify` 或 `push` 命令。

## Rust 模块划分

| Crate | 职责 |
| --- | --- |
| `ps-api` | Axum API 服务，保持现有 `/api/*` 契约 |
| `ps-cli` | 索引 fixture、scheduler、worker、notify、push 命令入口 |
| `ps-auth` | 认证、密码、令牌和 Cookie 兼容逻辑 |
| `ps-storage` | SQLite auth/index 存储访问 |
| `ps-index` | 索引 schema、写库、fixture parity 索引和变更清单 |
| `ps-sources` | Crossref/OpenAlex/Semantic Scholar/CNKI fixture source 解析 |
| `ps-recommend` | 通知候选、AI 选择、PushPlus 内容和状态文件逻辑 |
| `ps-worker` | scheduler 加载和通知/追踪分发编排 |
| `ps-domain` | 共享领域结构 |

## Python 参考模块

`paper_scanner/` 仍保留历史 Python 实现，以便：

- 契约测试比较 Rust 与 Python fixture 输出
- 保留历史解析、转换和 schema 行为的回归证据
- 支撑未迁移到 Rust 生产模式的离线测试夹具

不要把这些模块作为新的运行入口；正常开发和部署应使用 Rust 命令。

## 真实数据流

### 索引流

1. `ps-cli index fixture` 读取测试 CSV 和 recorded fixture
2. `ps-sources` 解析 source payload
3. `ps-index` 写入 `journals`、`issues`、`articles`、`article_listing`、`article_search`
4. 可选 `--manifest` 输出 `data/push_state/*.changes.json` 兼容清单

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

`dry-run` 和 `shadow` 不执行外部发送或收藏写入副作用。

## 本地运行

### Rust API

```bash
cargo run -p ps-api
```

默认后端地址：`http://127.0.0.1:8000`。

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

### Fixture 索引

```bash
cargo run -p ps-cli -- index fixture --csv tests/fixtures/contracts/scholarly/journals.csv --fixture tests/fixtures/contracts/scholarly/openalex_fallback_fixture.json --output-db data/index/scholarly-fixture.sqlite --manifest data/push_state/scholarly-fixture.changes.json
cargo run -p ps-cli -- index fixture --source cnki --csv tests/fixtures/contracts/cnki/journals.csv --fixture tests/fixtures/contracts/cnki/fixture.json --output-db data/index/cnki-fixture.sqlite
```

### 通知与追踪

```bash
cargo run -p ps-cli -- notify dry-run --auth-db data/auth.sqlite --index-db data/index/utd24.sqlite --db utd24.sqlite
cargo run -p ps-cli -- notify shadow --auth-db data/auth.sqlite --index-db data/index/utd24.sqlite --db utd24.sqlite --changes-file data/push_state/utd24.changes.json
cargo run -p ps-cli -- push dry-run --auth-db data/auth.sqlite --index-db data/index/utd24.sqlite --db utd24.sqlite
cargo run -p ps-cli -- push shadow --auth-db data/auth.sqlite --index-db data/index/utd24.sqlite --db utd24.sqlite --changes-file data/push_state/utd24.changes.json
```

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

Python 兼容测试改动：

```bash
uv run ruff check tests
uv run python -m unittest discover tests
```

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

### Python 模块不是运行入口

`paper_scanner/` 是兼容参考面。正常后端、worker、通知、追踪和 fixture 索引命令都应使用 Rust。

### 管理员定时任务不是 API 进程内 APScheduler

Rust worker sidecar 会加载任务配置；单次执行由 `ps-cli scheduler run-once TASK_ID` 触发，dry-run 由 `ps-cli scheduler dry-run-once TASK_ID` 触发。

### 通知配置不绑定单一 AI 服务商

通知链路使用 OpenAI 兼容接口。全局默认值来自 `NOTIFY_AI_*` 环境变量，用户级配置优先。
