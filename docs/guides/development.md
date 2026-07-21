# 开发指南

本文档面向修改 LitRadar 源码的贡献者，说明本地环境、日常开发流程、契约生成和质量检查。系统边界见[架构说明](../architecture.md)，测试层和诊断策略见[测试系统](../testing.md)，完整命令参数见 [CLI 参考](../reference/cli.md)。

## 工具链

CI 和容器使用以下主版本：

| 工具    | 版本/来源                                      |
| ------- | ---------------------------------------------- |
| Rust    | 1.96，workspace edition 2021                   |
| Node.js | 24                                             |
| pnpm    | 10.32.0                                        |
| Docker  | 当前 Docker Engine / Docker Desktop 与 Compose |

Rust 依赖由 `Cargo.lock` 锁定，前端依赖由 `app/pnpm-lock.yaml` 锁定。不要在普通开发任务中绕过 lockfile。

## 初始准备

### 部署密钥

本地后端也要求一个 32 字节原始密钥文件。`secrets/` 已被 Git 忽略：

```bash
mkdir -p secrets
openssl rand -out secrets/litradar.key 32
wc -c secrets/litradar.key
```

最后一条命令应输出 `32`。测试使用临时密钥和临时数据库，不应读取本机 `secrets/` 或仓库中的真实 `data/auth.sqlite`。

### 前端依赖

```bash
cd app
corepack enable pnpm
pnpm install --frozen-lockfile
```

## 运行开发服务

### 统一应用服务

在仓库根目录运行：

```bash
cargo run --bin litradar -- serve \
  --host 127.0.0.1 \
  --port 8001 \
  --secret-key-file secrets/litradar.key
```

该命令在一个进程中启动 HTTP 与内嵌调度；调度会立即执行一次 tick，之后默认每 30 秒检查一次。开发 HTTP 只在内部地址 `http://127.0.0.1:8001` 监听。启动下文的 Next.js 开发服务器后，浏览器统一通过以下 8000 端口地址访问：

- Web：`http://localhost:8000/`
- REST API：`http://localhost:8000/api`
- Swagger UI：`http://localhost:8000/docs/`
- OpenAPI：`http://localhost:8000/openapi.json`
- MCP：`http://localhost:8000/mcp`

服务端默认把 JSON Lines 写入 stderr；请求终态使用匹配 route、status、outcome、duration 和服务器生成的 request ID，不记录 query。成功健康检查和静态流量被抑制。本地阅读可改为 compact，并用 LitRadar 专用 filter 临时增加目标级别：

```bash
LITRADAR_LOG_FORMAT=compact \
LITRADAR_LOG_FILTER='warn,litradar=debug,litradar_api=debug' \
cargo run --bin litradar -- serve \
  --host 127.0.0.1 \
  --port 8001 \
  --secret-key-file secrets/litradar.key
```

PowerShell 需要先设置 `$env:LITRADAR_LOG_FORMAT = "compact"` 和 `$env:LITRADAR_LOG_FILTER = "warn,litradar=debug,litradar_api=debug"`，运行后用 `Remove-Item Env:LITRADAR_LOG_FORMAT, Env:LITRADAR_LOG_FILTER` 清理。配置、实际终端样式和隐私边界见[日志运维](../operations/logging.md)。

### 首个管理员

空用户库只能通过本机命令创建管理员：

```bash
printf '%s\n' "$ADMIN_PASSWORD" |
  cargo run --bin litradar -- admin bootstrap \
    --username admin \
    --password-stdin
```

该命令只在用户表为空时成功，不接受 `--password VALUE`。

### 内嵌调度

不需要第二个终端或独立调度服务。`litradar serve` 在同一生命周期内执行数据库中启用的类型化任务；可用 `--scheduler-interval-seconds N` 调整 tick 间隔。单次验证或触发使用 `litradar scheduler validate`、`litradar scheduler run-once` 或 `litradar scheduler dry-run-once`，具体语法见 [CLI 参考](../reference/cli.md)。

调度任务通过当前 `litradar` 可执行文件启动短生命周期子进程。SIGINT/SIGTERM 会取消正在运行的子进程、等待退出并保存 `cancelled` 状态；HTTP、心跳或调度组件意外失败会终止整个服务进程。

### 前端

```bash
cd app
pnpm dev
```

默认地址为 `http://localhost:8000`。`next.config.ts` 只在开发模式把同源 `/api/*`、`/mcp/*`、`/docs/*` 和 `/openapi.json` rewrite 到 `INTERNAL_API_URL`，默认 `http://localhost:8001`。只有浏览器需要跨源直连 API 时才设置 `NEXT_PUBLIC_API_URL`。

生产构建执行静态导出，rewrite 不会进入产物；Rust 直接从 `/app/web` 提供页面和压缩资源，并在同一 8000 监听器处理后端命名空间。

### 索引和投递

开发时优先选择单个小型 CSV 或离线 fixture。真实索引和投递会访问外部服务：

```bash
cargo run --bin litradar -- index \
  --secret-key-file secrets/litradar.key \
  --file chinese_journals.csv \
  --update

cargo run --bin litradar -- notify \
  --secret-key-file secrets/litradar.key \
  --dry-run
```

Scholarly 索引需要先在 `data/auth.sqlite` 的运行配置中保存 OpenAlex 和 Semantic Scholar key。通知 dry-run 仍会调用配置的 AI endpoint，但不会发送 PushPlus或写入收藏；完全确定性的开发检查应使用现有 fixture 测试。

## 修改位置

| 任务                | 主要位置                                                                |
| ------------------- | ----------------------------------------------------------------------- |
| 进程入口与生命周期  | `crates/litradar/src/`                                                  |
| REST 路由或 OpenAPI | `crates/litradar-api/src/routes/`、`crates/litradar-api/src/openapi.rs` |
| 认证                | `crates/litradar-auth/`、`crates/litradar-storage/src/auth.rs`          |
| 业务存储            | `crates/litradar-storage/src/business/`                                 |
| 数据库迁移          | `crates/litradar-storage/src/migrations.rs`                             |
| 索引和 schema       | `crates/litradar-index/`                                                |
| 上游数据源          | `crates/litradar-sources/`                                              |
| 推荐、通知和调度    | `crates/litradar-recommend/`、`crates/litradar-worker/`                 |
| 前端 API facade     | `app/lib/api/`、`app/lib/api.tsx`                                       |
| 前端页面和组件      | `app/app/`、`app/components/`                                           |
| 前端测试            | `app/tests/`                                                            |

## OpenAPI 与前端类型

库 crate `litradar-api` 是控制面 API schema 的来源；它不拥有可执行入口或 OS 信号。修改路由注解、DTO 或响应 schema 后，在 `app/` 运行：

```bash
pnpm generate:api
```

该命令：

1. 运行 Rust `litradar openapi` 子命令
2. 更新 `lib/generated/openapi.json`
3. 用 `openapi-typescript` 更新 `lib/generated/api-schema.tsx`
4. 格式化两个生成文件

CI 使用：

```bash
pnpm generate:api:check
```

认证、管理员任务、推送状态和秘密设置等关键响应还要经过 `app/lib/api-contract.tsx` 的运行时校验。不要用泛型断言替代这些边界。

## 数据库变更

认证库和索引库分别使用 `PRAGMA user_version`。修改 schema 时：

1. 在 `migrations.rs` 增加下一个有序版本。
2. 每个版本在独立事务内执行 DDL 和数据迁移。
3. 在同一事务末尾更新 `user_version`。
4. 覆盖空库、代表性旧库、当前版本幂等、失败回滚和未来版本拒绝。
5. 不在 repository 查询函数或连接 helper 中执行迁移。

索引新库的当前 schema 由 `litradar-index` 创建；storage migration 负责既有库升级。逻辑模型见[数据库参考](../reference/database.md)。

## 调度变更

定时任务是带 `kind` 的结构化 job，只允许 `index`、`notify` 和 `push`。内嵌调度器把已验证字段转换为当前 `litradar` 可执行文件加规范子命令的完整 argv，不调用 shell。

新增调度能力时必须同步更新：

- `litradar-domain` 的 job 类型
- API 和存储校验
- 内嵌调度的 argv 构造、运行认领、取消和持久状态
- OpenAPI 和前端管理界面
- 确定性 cron、时区、租约和失败测试

不要恢复自由命令字段、独立 worker 服务或按功能拆分的可执行文件。

## Rust 检查

日常从仓库根选择最低充分的统一入口；完整职责和聚焦命令见[测试系统](../testing.md)：

```bash
node scripts/test.mjs fast
node scripts/test.mjs integration
node scripts/test.mjs all
```

Backend CI 使用固定的 cargo-nextest 0.9.137、零重试和独立 doctest。`cargo test --workspace --locked` 保留为完整计划或发布前的一次 Cargo 兼容门禁，不在每个 PR 中与 nextest 重复。

覆盖率只在每周/手动诊断中分别生成 Rust 和前端报告，不设阈值：

```bash
node scripts/test.mjs diagnostics
```

## 前端检查

聚焦前端时可在 `app/` 直接运行：

```bash
cd app
pnpm generate:api:check
pnpm lint
pnpm format:check
pnpm exec tsc --noEmit
pnpm test:unit
pnpm exec playwright install --with-deps chromium
pnpm test:browser-components
pnpm test:e2e:fixtures
pnpm test:e2e:full-stack
pnpm build
```

Vitest/jsdom 使用显式 MSW 场景；Browser Mode 只验证焦点、Clipboard 和 IntersectionObserver 等原生语义。Playwright fixture 项目保留 7 条拦截式 UI smoke；full-stack 项目构建前端并通过实际 Rust listener、HttpOnly Cookie 和临时 SQLite 运行 3 条无请求拦截的关键旅程。CI 最多重试 Playwright 一次以取得 trace/video，但 retry-pass 仍按 flaky 失败。

## 部署检查

修改 Docker 或 Compose 时至少运行：

```bash
docker compose config --quiet
docker compose build
docker build --tag litradar:test .
node scripts/container-smoke.mjs litradar:test
```

根 Dockerfile 必须成功导出前端并把 `out/` 复制到最终 Debian 层。最终镜像只复制 release `litradar`，必须没有其他应用可执行文件、Node.js/standalone 运行时并保持非 root；根 Compose 只能声明一个 `litradar` 服务。只读根文件系统、tmpfs、显式数据卷、空 capability 集合、`no-new-privileges`、健康检查和重启策略都是部署契约。

日志或请求路径变更还应使用隔离 fixture 运行 off/on 门禁：

```powershell
pwsh ./scripts/profile_logging.ps1 -DataPath ./output/logging-fixture -Rounds 3 -RequestCount 300 -Concurrency 4
```

脚本验证 JSON schema、请求事件完整性、零丢失、p95 延迟差，并复用 Docker warm-idle 内存画像。它会迁移和写入传入目录，不能指向正在运行的真实数据。

## 测试边界

完整放置规则、共享场景限制、功能所有权和报告路径见[测试系统](../testing.md)。

- 后端测试使用临时目录、临时 SQLite、临时密钥和 fixture transport。
- 不对仓库真实 `data/` 执行备份恢复、密文迁移或写入。
- 时间相关测试传入确定性时间值，不使用长时间 sleep。
- 上游 HTML/JSON 解析使用 replay 或 fixture；凭据失效不是读取本机生产数据库的理由。
- 并发和调度测试通过唯一约束、租约和可控时钟验证，不依赖偶然执行顺序。

## 常见误区

- `--notify-dry-run` 只决定 `index --notify` 的下游模式；要发生 handoff，必须同时使用 `--update --notify`。
- `push` 的默认状态目录是 `data/folder_push_state`，不是 `data/push_state`。
- 每周更新和投递依赖 `*.changes.json`，不是按文章日期实时扫描。
- 前端 API 入口是 `app/lib/api.tsx` 和 `app/lib/api/`。
- `INTERNAL_API_URL` 只控制本地 `next dev` 的内部代理目标，不是生产容器配置。
- 全局 scholarly key 池与用户级 AI/PushPlus 设置是两套不同配置。
