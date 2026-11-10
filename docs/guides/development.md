# 开发指南

本文档面向修改 LitRadar 源码的贡献者，说明本地环境、日常开发流程、契约生成和质量检查。系统边界见[架构说明](../architecture/README.md)，测试层和诊断策略见[测试系统](testing.md)，完整命令参数见 [CLI 参考](../reference/cli.md)。

## 工具链

CI 和容器使用以下主版本：

| 工具    | 版本/来源                                      |
| ------- | ---------------------------------------------- |
| Go      | 1.27.3，CGO_ENABLED=1，需 C 编译器             |
| Node.js | 26.11.1                                        |
| pnpm    | 12.16.1                                        |
| Docker  | 当前 Docker Engine / Docker Desktop 与 Compose |

Go 依赖由 `go.mod` / `go.sum` 与 `third_party/` 的固定补丁锁定，前端依赖由根目录 `pnpm-lock.yaml` 锁定。不要在普通开发任务中绕过 lockfile。

Go 命令固定使用 `CGO_ENABLED=1`、`GOWORK=off`、`GOENV=off`、`GOFLAGS=""`、`GOTOOLCHAIN=go1.27.3`。Windows 原生构建要求 PATH 中有匹配的 MinGW GCC；生产镜像直接使用 Obscura 官方 v0.2.4 的 render + stealth 二进制及配套 worker。

pnpm 会转发已有的 `NODE_OPTIONS`，并为子进程关闭 Node.js 实验性 Web Storage，使测试中的 jsdom 浏览器存储与 Node.js 宿主存储隔离。

## 初始准备

以下 Bash 命令在仓库根目录执行，除非步骤明确要求进入 `apps/client/frontend/`。Windows 可使用 WSL Bash，或把命令改为当前 PowerShell 的等价语法；标为 PowerShell 的画像命令需要 PowerShell 7。

### 部署密钥

本地后端要求一个 32 字节原始密钥文件。`secrets/` 已被 Git 忽略；已有数据必须使用原密钥，不能重新生成并覆盖：

```bash
mkdir -p secrets
if [ ! -e secrets/litradar.key ]; then
  (umask 077; openssl rand -out secrets/litradar.key 32)
fi
wc -c secrets/litradar.key
```

最后一条命令输出的字节数应为 `32`，后面还会显示文件名。测试使用临时密钥和临时数据库，不应读取本机 `secrets/` 或仓库中的真实 `data/auth.sqlite`。

### 前端依赖

```bash
cd apps/client/frontend
npm install --global pnpm@12.16.1
pnpm install --frozen-lockfile
```

## 运行开发服务

### 一条命令启动前后端

完成部署密钥和前端依赖准备后，在仓库根目录运行：

```bash
pnpm dev
```

根目录 `pnpm dev` 调用 `scripts/dev.mjs`。脚本检查部署密钥和 8000/8001 端口，使用锁定依赖编译并启动 Go 开发服务，同时启动 Vite，最多等待 5 分钟确认两个服务就绪。浏览器入口为 `http://localhost:8000`，按 `Ctrl+C` 关闭前后端；任一服务启动失败或意外退出时，脚本也会停止另一服务。退出最多给予子进程 10 秒宽限，再终止仍存活的进程树。端口已被占用时直接报错，不终止已有服务。

该命令不需要前端发布构建、静态资源目录或目录连接。默认使用仓库中的数据和部署密钥；需要隔离数据时，使用 `node scripts/dev.mjs --project-root PATH`，该目录必须已有 `secrets/litradar.key`，源码和前端依赖仍从当前仓库读取。脚本不会生成或替换部署密钥。

### 统一应用服务

在仓库根目录运行：

```bash
go run -mod=readonly -tags sqlite_fts5,sqlite_dbstat ./apps/server serve \
  --development \
  --host 127.0.0.1 \
  --port 8001 \
  --secret-key-file secrets/litradar.key
```

该命令在一个进程中启动 HTTP 与内嵌调度；调度会立即执行一次 tick，之后默认每 30 秒检查一次。开发 HTTP 只在内部地址 `http://127.0.0.1:8001` 监听。启动下文的 Vite 开发服务器后，浏览器统一通过以下 8000 端口地址访问：

- Web：`http://localhost:8000/`
- REST API：`http://localhost:8000/api`
- Swagger UI：`http://localhost:8000/docs/`
- OpenAPI：`http://localhost:8000/openapi.json`
- MCP：`http://localhost:8000/mcp`

服务端始终只托管 API、认证、健康检查、接口文档、MCP 与内嵌任务；直接访问页面路径返回 404。`--development` 只允许 loopback，且不能与 `--require-secure-cookies` 组合；不传该参数也不需要 UI 构建。

服务端默认把 JSON Lines 写入 stderr；请求终态使用匹配 route、status、outcome、duration 和服务器生成的 request ID，不记录 query。成功健康检查和静态流量被抑制。日志格式和 filter 是管理员“运行配置”中的持久设置，不接受进程环境覆盖：首次按默认 JSON 启动，登录管理页把 `log_format` 改为 `compact`，按需把 `log_filter` 改为例如 `warn,litradar=debug,litradar_api=debug`，再重启进程。配置、实际终端样式和隐私边界见[日志运维](../operations/logging.md)。

### 首个管理员

空用户库只能通过本机命令创建管理员：

```bash
IFS= read -r -s -p 'Admin password: ' ADMIN_PASSWORD
printf '\n'
printf '%s\n' "$ADMIN_PASSWORD" |
  go run -mod=readonly -tags sqlite_fts5,sqlite_dbstat ./apps/server admin bootstrap \
    --username admin \
    --password-stdin
unset ADMIN_PASSWORD
```

该命令只在用户表为空时成功，不接受 `--password VALUE`。

### 内嵌调度

不需要第二个终端或独立调度服务。`litradar serve` 在同一生命周期内执行数据库中启用的类型化任务；可用 `--scheduler-interval-seconds N` 调整 tick 间隔。单次验证或触发使用 `litradar scheduler validate`、`litradar scheduler run-once` 或 `litradar scheduler dry-run-once`，具体语法见 [CLI 参考](../reference/cli.md)。

调度任务通过当前 `litradar` 可执行文件启动短生命周期子进程。SIGINT/SIGTERM 会取消正在运行的子进程、等待退出并保存 `cancelled` 状态；HTTP、心跳或调度组件意外失败会终止整个服务进程。

### 前端

```bash
cd apps/client/frontend
pnpm dev
```

默认地址为 `http://localhost:8000`。`vite.config.ts` 将 `/api`、`/mcp`、`/docs`、`/openapi.json` 与 `/health` 代理到内部 Go 8001。生产部署由 Caddy 执行同源路由。

在根目录执行 `pnpm build` 生成 `apps/client/frontend/dist`；`node apps/server/scripts/build-go.mjs` 仅构建服务端，不读取客户端产物。Web 客户端在 `apps/client` 执行 `task build:server`，输出 `bin/litradar-client-web`，`task package:web` 再打包为 `bin/litradar-client-web_<version>_linux_amd64.tar.gz`；Windows 执行 `task package:windows`，Android 执行 `task package:android`。平台与签名要求见[发布说明](../operations/releases.md)。

### 索引和投递

开发时优先选择单个小型 CSV 或离线 fixture。真实索引和投递会访问外部服务：

```bash
go run -mod=readonly -tags sqlite_fts5,sqlite_dbstat ./apps/server index \
  --secret-key-file secrets/litradar.key \
  --file chinese_journals.csv \
  --update

go run -mod=readonly -tags sqlite_fts5,sqlite_dbstat ./apps/server notify \
  --secret-key-file secrets/litradar.key \
  --dry-run
```

Scholarly 索引需要先在管理后台配置 Crossref 联系邮箱、OpenAlex 和 Semantic Scholar 密钥池，字段说明见[运行配置](../reference/configuration.md)。管理员页按已发现的 CSV 或数据库名称提供索引 Provider 单选，以及摘要页、全文的继承、排序和显式禁用控件；选项由后端能力目录过滤。

通知 dry-run 仍会调用配置的 AI Endpoint，但不会发送 PushPlus 或写入收藏；完全确定性的开发检查应使用现有 fixture 测试。

## 修改位置

| 任务                 | 主要位置                                                             |
| -------------------- | -------------------------------------------------------------------- |
| 进程入口与生命周期   | `apps/server/`、`apps/server/internal/cli/`、`apps/server/internal/runtime/`                |
| REST 路由或 OpenAPI  | `apps/server/internal/api/`、`apps/server/internal/openapi/`                                 |
| 认证                 | `apps/server/internal/domain/auth/`、`apps/server/internal/storage/auth/`                    |
| 业务存储             | `apps/server/internal/storage/`                                                  |
| 数据库迁移与内容定义 | `apps/server/internal/storage/migrations/auth/`、`apps/server/internal/storage/indexschema/` |
| 索引执行与控制状态   | `apps/server/internal/index/`                                                    |
| 上游数据源           | `apps/server/internal/sources/`                                                  |
| 推荐、通知和调度     | `apps/server/internal/delivery/`、`apps/server/internal/scheduler/`                          |
| 前端 API facade      | `apps/client/frontend/lib/api/`、`apps/client/frontend/lib/api.tsx`                                    |
| 前端页面和组件       | `apps/client/frontend/src/routes/`、`apps/client/frontend/components/`                                        |
| 前端测试             | `apps/client/frontend/tests/`                                                         |

## OpenAPI 与前端类型

`apps/server/internal/openapi/` 中的声明及 `apps/server/internal/api/` 的真实路由绑定是控制面 API schema 的来源；它不拥有可执行入口或 OS 信号。修改路由、DTO 或响应 schema 后，在 `apps/client/frontend/` 运行：

```bash
pnpm generate:api
```

该命令：

1. 运行 Go `litradar openapi` 子命令
2. 更新 `lib/generated/openapi.json`
3. 用 `openapi-typescript` 更新 `lib/generated/api-schema.tsx`
4. 格式化两个生成文件

CI 使用：

```bash
pnpm generate:api:check
```

认证、管理员任务、推送状态和秘密设置等关键响应还要经过 `apps/client/frontend/lib/api-contract.tsx` 的运行时校验。不要用泛型断言替代这些边界。

## 数据库变更

认证库和内容库分别使用 `PRAGMA user_version`，但迁移事务的组织方式不同。认证库在 `apps/server/internal/storage/migrations/auth/` 按有序版本逐个执行，每个版本独立提交，并在同一事务末尾更新版本号；内容库由 `apps/server/internal/storage/indexschema/` 统一定义 DDL、版本和校验规则，迁移可在一个事务中完成多步转换，最后更新到目标版本。

修改前先确定数据库类型，再更新相应定义与迁移路径。测试应覆盖空库、代表性旧库、当前版本幂等、失败回滚和未来版本拒绝；不要在查询函数或连接辅助函数中隐式执行迁移。

`apps/server/internal/index` 使用共享内容定义初始化和写入新库，storage 负责既有库的迁移与预检。兼容版本、显式离线优化和控制库边界见[数据库参考](../reference/database.md)。

## 调度变更

定时任务是带 `kind` 的结构化 job，只允许 `index`、`notify` 和 `push`。内嵌调度器把已验证字段转换为当前 `litradar` 可执行文件加规范子命令的完整 argv，不调用 shell。

新增调度能力时必须同步更新：

- `apps/server/internal/domain` 的 job 类型
- API 和存储校验
- 内嵌调度的 argv 构造、运行认领、取消和持久状态
- OpenAPI 和前端管理界面
- 确定性 cron、时区、租约和失败测试

不要恢复自由命令字段、独立 worker 服务或按功能拆分的可执行文件。

## Go 检查

日常从仓库根选择最低充分的统一入口；完整职责和聚焦命令见[测试系统](testing.md)：

```bash
node tests/test.mjs fast
node tests/test.mjs integration
node tests/test.mjs all
```

Backend CI 在 Windows 和 Linux 执行 `node apps/server/scripts/check-go.mjs`，固定 Go 1.27.3、CGO 与两个 SQLite tags，运行格式、模块/补丁完整性、vet、无缓存常规和 race 测试，以及两个补丁依赖在自身模块和根模块中的测试。全模块常规与 race 测试每条最多 25 分钟（单包 `-timeout=20m`），其他命令每条最多 15 分钟；失败即停止，不会自动重试。

覆盖率只在每周/手动诊断中分别生成 Go 和前端报告，不设阈值：

```bash
node tests/test.mjs diagnostics
```

## 前端检查

聚焦前端时可在 `apps/client/frontend/` 直接运行：

```bash
cd apps/client/frontend
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

Vitest/jsdom 使用显式 MSW 场景；Browser Mode 验证焦点、Clipboard、IntersectionObserver 和动效等原生语义。Playwright fixture 项目负责拦截式 UI 冒烟测试；full-stack 项目构建前端，通过真实 Go 监听器、HttpOnly Cookie 和临时 SQLite 验证关键旅程。具体覆盖以[测试系统](testing.md)和当前测试文件为准。CI 最多重试 Playwright 一次以取得 trace/video，但重试后通过仍按不稳定测试判定失败。

## 部署检查

修改 Docker 或 Compose 时至少运行：

```bash
docker compose config --quiet
docker compose build
docker build -f apps/server/Dockerfile --tag litradar:test .
node apps/server/tests/container-smoke.mjs litradar:test
```

服务端 Dockerfile 仅构建 Go 服务端及原生辅助程序；Web 客户端使用 `apps/client/build/docker/Dockerfile.server`，构建前先在根目录执行 `pnpm build`。Compose 包含 server、client-web、gateway；只有 server 访问数据和密钥。

日志或请求路径变更还应使用隔离 fixture 运行 off/on 门禁：

```powershell
pwsh ./apps/server/tests/profiling/profile_logging.ps1 -DataPath ./output/logging-fixture -Rounds 3 -RequestCount 300 -Concurrency 4
```

脚本验证 JSON schema、请求事件完整性、零丢失、p95 延迟差，并复用 Docker warm-idle 内存画像。它会迁移和写入传入目录，不能指向正在运行的真实数据。

## 测试边界

完整放置规则、共享场景限制、功能所有权和报告路径见[测试系统](testing.md)。

- 后端测试使用临时目录、临时 SQLite、临时密钥和 fixture transport。
- 不对仓库真实 `data/` 执行备份恢复、密文迁移或写入。
- 时间相关测试传入确定性时间值，不使用长时间 sleep。
- 上游 HTML/JSON 解析使用 replay 或 fixture；凭据失效不是读取本机实际数据库的理由。
- 并发和调度测试通过唯一约束、租约和可控时钟验证，不依赖偶然执行顺序。

## 常见误区

- `--notify-dry-run` 只决定 `index --notify` 的下游模式；要发生 handoff，必须同时使用 `--update --notify`。
- notify/push 的可变状态都在 `data/auth.sqlite`；`data/push_state/*.changes.json` 只是候选输入，两个旧状态目录只用于一次性只读导入。
- 每周更新和投递依赖 `*.changes.json`，不是按文章日期实时扫描。
- 前端 API 入口是 `apps/client/frontend/lib/api.tsx` 和 `apps/client/frontend/lib/api/`。
- 前端 API 始终同源；本地 Go 服务需要监听固定的 `127.0.0.1:8001` 才能被 `pnpm dev` 代理。
- 全局 scholarly key 池与用户级 AI/PushPlus 设置是两套不同配置。

<a id="weekly-manifest-cache-verification"></a>

## 每周更新缓存验证

API 在汇总与文章分页请求之间共享解析后的每周更新清单。缓存最多保存 64 份发布清单和 1,000,000 个文章 ID，条目在 60 秒后过期，复用前核对规范路径、文件长度和时间戳。文件变化或损坏时不会返回旧缓存；超大清单仍可读取，但不会驻留缓存。缓存只含来源元数据，不含账号凭据。

修改相关逻辑时，在仓库根运行正确性与确定性过期测试：

```bash
go test -count=1 -race -mod=readonly -tags sqlite_fts5,sqlite_dbstat ./apps/server/internal/storage/weekly ./apps/server/internal/storage/query
```

Go 版本的容器性能画像使用 `node apps/server/tests/profiling/go-image.mjs`，要求已构建 `litradar:go-test-amd64`。

历史记录中，2026-09-05 的一次本地 Windows release 测试使用 8 个目录、每个目录 10,000 篇文章。10 次未缓存的续页请求总计 267.83 ms，预热缓存后为 198.03 ms；预热后的系列在初始 8 次解析之外没有新增解析。这是单次合成测试结果，仅说明当时的测试表现，不构成线上延迟承诺。目录元数据检查、成员分组和每次查询的临时 SQLite 成员表仍有开销。
