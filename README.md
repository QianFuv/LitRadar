# Paper Scanner

Paper Scanner 是一个面向学术期刊的全栈检索与订阅平台。它负责从 Crossref、OpenAlex、Semantic Scholar 与 CNKI overseas 抓取期刊和文章元数据，构建 SQLite 检索库，并提供 Web 界面、收藏夹、追踪推送、每周更新、公告与后台管理能力。

当前后端运行路径已经切换到 Rust，保留原来的用户命令名：

- `api`：启动兼容现有 `/api/*` 契约的 Rust API 后端
- `admin`：在本机通过 stdin 初始化首个管理员，后续承载离线维护命令
- `index`：读取 `data/meta/*.csv`，抓取上游元数据并写入 `data/index/*.sqlite`
- `notify`：执行或演练 PushPlus 通知链路
- `push`：执行或演练追踪文件夹写入链路
- `scheduler`：校验或手动触发管理员类型化定时任务
- `worker`：启动 Rust worker sidecar，周期性加载并直接执行启用的 `index`、`notify`、`push` job

## 主要功能

- 多数据源索引：英文期刊使用 Crossref + OpenAlex + Semantic Scholar，中文期刊使用 CNKI overseas
- SQLite 检索：文章全文检索基于 FTS5，可选加载 `simple` 中文分词扩展
- 多维筛选：按期刊、领域、年份、开放获取等条件过滤
- 每周更新：基于变更清单聚合最近新增文章
- 用户系统：注册、登录、邀请码、访问令牌、改密
- 收藏与导出：文件夹管理、批量收藏、BibTeX / RIS / EndNote XML 导出
- 文献追踪：将某个文件夹设为追踪文件夹，并按用户偏好自动写入相关文章
- MCP：Rust API 直接提供 Streamable HTTP MCP 端点，不需要额外 MCP 运行时
- AI 选择：支持 OpenAI 兼容模型配置，不局限于单一服务商
- 管理后台：用户、邀请码、系统统计、外部元数据运行配置、定时任务、系统公告
- 首页公告：后台可配置全局公告，前台按优先级展示并支持本地关闭

## 技术栈

| 层级 | 组件 |
| --- | --- |
| 前端 | Next.js 16、React 19、TypeScript、Tailwind CSS 4、Radix UI、TanStack Query |
| 后端 | Rust、Axum、Tokio、rusqlite |
| 索引/抓取 | Rust workspace crates、SQLite FTS5 |
| AI 与推送 | OpenAI 兼容服务、PushPlus |
| 调度 | Rust worker/CLI |
| 开发工具 | Cargo、pnpm、Docker |

## 仓库结构

```text
.
├── app/                     前端项目
├── crates/                  Rust 后端 workspace
├── docs/                    详细文档
├── data/
│   ├── meta/                期刊 CSV 元数据源
│   ├── index/               生成后的 SQLite 检索库
│   ├── push_state/          通知与每周更新状态、变更清单
│   └── auth.sqlite          用户、收藏、通知、管理员数据
├── libs/                    SQLite simple tokenizer 预编译扩展
├── docker-compose.yml       根 Docker Compose 编排
├── Dockerfile               后端镜像构建
└── Cargo.toml               Rust workspace 配置
```

## 快速开始

### 方式一：Docker Compose

前提：

- 已安装 Docker 与 Docker Compose

步骤：

1. 准备期刊 CSV

   仓库已自带示例 CSV，可直接使用 `data/meta/*.csv`。每个 CSV 默认包含以下列：

   | 列名 | 说明 |
   | --- | --- |
   | `source` | 数据源；英文期刊为 `scholarly`，中文期刊为 `cnki` |
   | `title` | 期刊标题 |
   | `issn` | ISSN |
   | `id` | 上游期刊 ID；`scholarly` 使用 ISSN，`cnki` 使用 CNKI `pykm` |
   | `area` | 自定义领域标签 |

2. 构建并启动 Rust API、Rust worker 与前端服务

   Linux 原生 Docker Engine 上，后端镜像以固定 UID/GID `10001:10001` 运行。首次挂载或迁移既有数据前，先让该账号拥有数据目录；Docker Desktop for macOS/Windows 通常不需要这一步：

   ```bash
   sudo chown -R 10001:10001 data
   ```

   ```bash
   docker compose build
   docker compose up -d
   ```

3. 准备索引数据库

   Docker 运行时读取宿主机 `data/index/*.sqlite`。如果该目录已有生产或测试索引库，可直接启动服务；需要从 `data/meta/*.csv` 重新生成时，可运行：

   ```bash
   docker compose run --rm api index --file english_journals.csv --update --notify-dry-run
   ```

   `--file` 省略时会处理 `data/meta/` 下所有 CSV。

4. 访问服务

   - 前端：`http://localhost:3000`
   - 后端 API：`http://localhost:8000/api`
   - HTTP MCP：`http://localhost:8000/mcp`
   - API 文档：`http://localhost:8000/docs/`

5. 在本机初始化第一个管理员

   公开注册不会创建第一个用户。请通过标准输入把密码交给容器内的本地维护命令：

   ```bash
   printf '%s\n' "$ADMIN_PASSWORD" | docker compose run --rm -T api admin bootstrap --username admin --password-stdin
   ```

   `ADMIN_PASSWORD` 应由当前 shell 的安全输入或密码管理器提供，不要把实际密码写进命令历史。初始化成功后，管理员登录并生成邀请码；所有公开注册都必须提交有效邀请码。

### 方式二：本地开发

#### 后端

```bash
cargo run --bin api
```

默认后端地址：`http://127.0.0.1:8000`

首次本地启动还需要在另一个终端通过 stdin 创建管理员：

```bash
printf '%s\n' "$ADMIN_PASSWORD" | cargo run --bin admin -- bootstrap --username admin --password-stdin
```

交互式 API 文档地址：`http://127.0.0.1:8000/docs/`，生成的 OpenAPI JSON 地址：`http://127.0.0.1:8000/openapi.json`。

默认启动会输出 HTTP 请求日志，包含 method、path、status 和 latency。可通过 `RUST_LOG` 调整过滤级别，例如：

```bash
RUST_LOG=error cargo run --bin api
```

另开一个终端可启动 Rust worker 进程：

```bash
cargo run --bin worker -- --interval-seconds 30
```

回归测试和覆盖率检查使用 Rust workspace 命令，见下方开发文档。常用覆盖率摘要：

```bash
cargo llvm-cov --workspace --summary-only
```

#### 前端

```bash
cd app
corepack enable pnpm
pnpm install
pnpm dev
```

默认前端地址：`http://localhost:3000`

如果前后端分离运行，前端可通过 `NEXT_PUBLIC_API_URL` 指定 API 根地址。

Rust OpenAPI 是前端控制面类型的唯一来源。修改 API DTO 或路由注解后，在 `app/` 运行 `pnpm generate:api`，并提交 `lib/generated/openapi.json` 与 `lib/generated/api-schema.tsx`。前端质量检查使用 `pnpm lint`、`pnpm format:check`、`pnpm exec tsc --noEmit`、`pnpm test`、`pnpm test:e2e` 和 `pnpm build`。

## 核心命令

### 1. 索引

```bash
cargo run --bin index -- --file english_journals.csv --update
cargo run --bin index -- --file cnki_journals.csv --resume --issue-batch 10
```

`index` 参数：

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `--file`, `-f` | 空 | 只处理 `data/meta/` 下指定 CSV；省略时处理全部 CSV |
| `--workers`, `-w` | `32` | 每个期刊 worker 内的 CNKI 文章详情请求并发数；省略 `--issue-batch` 时也作为 issue 批大小 |
| `--processes` | `2` | 单个 CSV 内的期刊 worker 进程数；多个 CSV 仍逐个处理 |
| `--issue-batch` | 同 `--workers` | CNKI 每轮合并的 issue 数，用于文章详情调度和写入 |
| `--timeout` | `20` | 上游请求超时秒数 |
| `--resume` / `--no-resume` | `--resume` | 跳过已完成期刊或年份 |
| `--update` / `--no-update` | `--no-update` | 增量更新并生成 `data/push_state/*.changes.json` |
| `--notify` | `false` | 更新后调用 `notify` |
| `--notify-dry-run` | `false` | `--notify` handoff 使用 dry-run |

CNKI 路径在每个期刊 worker 内顺序拉取 issue 列表，把当前 issue batch 内的文章详情按 `--workers` 并发抓取。英文 scholarly 路径不使用 `--workers` 扩大请求并发；Semantic Scholar batch 请求会按 `--processes` 做进程感知错峰限速。

OpenAlex、Semantic Scholar 和 Crossref 的共享运行配置由管理员后台的运行时配置表维护。索引命令启动时从 `data/auth.sqlite` 读取这些配置，不读取进程环境变量。

### 2. API 服务

```bash
cargo run --bin api -- --host 127.0.0.1 --port 8000 --project-root .
```

API 启动后会提供：

- `/api/*`：业务接口
- `/mcp`：Streamable HTTP MCP 协议端点，复用 API Bearer Token 或 `ps_session` Cookie 认证
- `/docs/`：Swagger UI 交互式文档
- `/openapi.json`：由 Rust 注解和 DTO schema 编译期生成的 OpenAPI JSON

终端默认显示请求日志；设置 `RUST_LOG=error` 可只显示 error 级日志。

启动参数：

- `--host`：监听地址，默认 `127.0.0.1`
- `--port`：监听端口，默认 `8000`
- `--project-root`：项目根目录，默认当前目录；Docker 中为 `/app`
- `--require-secure-cookies`：生产启动门；数据库中的 `secure_cookies` 不是 `true` 时拒绝启动

共享运行配置通过管理员后台写入 `data/auth.sqlite` 的 `runtime_settings` 表。当前受管理的配置项为：

| 配置项 | 说明 |
| --- | --- |
| `openalex_api_key_pool` | OpenAlex API key 池；scholarly 索引需要 |
| `semantic_scholar_api_key_pool` | Semantic Scholar API key 池；scholarly 索引需要 |
| `crossref_mailto_pool` | Crossref 联系邮箱池，建议生产环境配置 |
| `cors_allowed_origins` | 允许跨源浏览器访问的 Origin 列表 |
| `mcp_allowed_hosts` | HTTP MCP `Host` 白名单 |
| `mcp_allowed_origins` | HTTP MCP 浏览器 `Origin` 白名单 |
| `secure_cookies` | `ps_session` Cookie 是否带 `Secure` 标记 |

API、索引器和调度任务启动时会读取 `runtime_settings`。Docker Compose 只挂载 `./data:/app/data`，因此可以复用现有 `data/auth.sqlite` 中的运行时配置。

管理员初始化只能通过本机 `admin bootstrap --username NAME --password-stdin` 完成。该命令只接受标准输入密码，并且仅在用户表为空时成功。公开注册始终需要邀请码，新注册、改密和管理员重置密码的最小长度为 12 个字符；既有密码仍可直接登录。

### 3. PushPlus 通知推送

```bash
cargo run --bin notify -- --dry-run
cargo run --bin notify -- --db utd24.sqlite --changes-file data/push_state/utd24.changes.json --no-dry-run
```

该命令只处理 `delivery_method = "pushplus"` 的用户。`--dry-run` 不发送消息；省略 `--db` 时会处理 `data/index/*.sqlite`。

### 4. 追踪文件夹推送

```bash
cargo run --bin push -- --dry-run
cargo run --bin push -- --db utd24.sqlite --changes-file data/push_state/utd24.changes.json --no-dry-run
```

该命令只处理 `delivery_method = "folder"` 且已配置追踪文件夹的用户。`--dry-run` 不写入收藏；省略 `--db` 时会处理 `data/index/*.sqlite`。

## AI 与推送配置

用户级通知/追踪偏好保存在 `data/auth.sqlite` 的 `notification_settings` 表中，可通过前端“文献追踪”页面或 `/api/tracking/notification-settings` API 配置。

AI、PushPlus 与投递方式是用户级设置。用户可在“文献追踪”页面或 `/api/tracking/notification-settings` API 中配置主备 OpenAI 兼容 endpoint、模型、API key、系统提示词、PushPlus token、模板、topic、channel 与候选上限。未配置用户级 AI key/model 时，该用户会被跳过；通知链路不读取进程环境变量作为默认凭据。

## 数据与状态文件

### 索引数据库

- 路径：`data/index/<csv_stem>.sqlite`
- 来源：每个 `data/meta/*.csv`
- 主要表：`journals`、`journal_meta`、`issues`、`articles`、`article_listing`、`article_search`、`listing_state`、`journal_year_state`、`journal_state`、`index_runs`、`index_path_stats`、`index_api_call_stats`

### 用户数据库

- 路径：`data/auth.sqlite`
- 主要表：`users`、`access_tokens`、`cnki_sessions`、`folders`、`favorites`、`invite_codes`、`notification_settings`、`scheduled_tasks`、`runtime_settings`、`announcements`

### 变更与推送状态

- `data/push_state/<db>.changes.json`：索引增量更新时生成的变更清单
- `data/push_state/<db>.json`：PushPlus 通知和追踪文件夹推送流水状态

说明：

- `/api/weekly-updates`
- `/api/tracking/push-weekly`
- `notify --changes-file ...`
- `push --changes-file ...`

这几条链路都依赖 `*.changes.json` 文件，并只把顶层变更字段作为运行时输入；`summary` 仅用于计数与诊断信息。

## 部署说明

根目录 `docker-compose.yml` 使用三个服务：

- `api`：Rust API 后端，默认仅绑定宿主机 `127.0.0.1:8000`
- `worker`：Rust worker 进程，复用后端镜像并挂载同一个 `data` 目录
- `app`：前端，默认仅绑定宿主机 `127.0.0.1:3000`

三个运行容器都使用非 root 用户、只读根文件系统、独立 `/tmp` tmpfs、`no-new-privileges`、空 Linux capability 集合、健康检查和 `restart: unless-stopped`。API 健康检查访问 `/api/health`；worker 健康检查通过 `/api/health/worker` 读取持久心跳；前端健康检查同时验证 Next.js 与内部 API rewrite。公网部署应在 TLS 反向代理后运行，并用 `--require-secure-cookies` 强制数据库配置与 HTTPS 边界一致。

前端在 Docker 构建阶段使用 `INTERNAL_API_URL` 将 `/api/*` 重写到后端；`app/Dockerfile` 默认为 `http://api:8000`。根 Compose 文件里没有显式设置这个变量，是因为 `app/Dockerfile` 已提供该默认值。

当前主前端登录流程只使用后端 `/api/auth/*`。仓库已移除旧的前端令牌认证工具与 `config` 挂载；根 Compose 现在只依赖 `data` 卷。Rust 服务读取现有 `data/index/*.sqlite`、`data/auth.sqlite` 与推送状态文件。

## 详细文档

- [API 文档](docs/api.md)
- [数据库说明](docs/database.md)
- [开发指南](docs/development.md)
- [Docker 部署](docs/docker.md)
- [安全说明](docs/security.md)
- [通知与追踪推送](docs/notify.md)
- [Crossref / OpenAlex / Semantic Scholar 集成](docs/scholarly_api.md)
- [CNKI overseas 集成](docs/cnki_oversea_api.md)
- [前端说明](app/README.md)
