# Paper Scanner

Paper Scanner 是一个面向学术期刊的全栈检索与订阅平台。它负责从 Crossref、OpenAlex、Semantic Scholar 与 CNKI overseas 抓取期刊和文章元数据，构建 SQLite 检索库，并提供 Web 界面、收藏夹、追踪推送、每周更新、公告与后台管理能力。

当前后端运行路径已经切换到 Rust，保留原来的用户命令名：

- `api`：启动兼容现有 `/api/*` 契约的 Rust API 后端
- `index`：读取 `data/meta/*.csv`，抓取上游元数据并写入 `data/index/*.sqlite`
- `notify`：执行或演练 PushPlus 通知链路
- `push`：执行或演练追踪文件夹写入链路
- `ps-cli worker shadow`：启动 Rust worker sidecar，周期性加载定时任务并保持服务运行

## 主要功能

- 多数据源索引：英文期刊使用 Crossref + OpenAlex + Semantic Scholar，中文期刊使用 CNKI overseas
- SQLite 检索：文章全文检索基于 FTS5，可选加载 `simple` 中文分词扩展
- 多维筛选：按期刊、领域、年份、开放获取等条件过滤
- 每周更新：基于变更清单聚合最近新增文章
- 用户系统：注册、登录、邀请码、访问令牌、改密
- 收藏与导出：文件夹管理、批量收藏、BibTeX / RIS / EndNote XML 导出
- 文献追踪：将某个文件夹设为追踪文件夹，并按用户偏好自动写入相关文章
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
   - API 文档：`http://localhost:8000/docs/`

5. 注册第一个用户

   第一个注册用户不需要邀请码，并会自动成为管理员。之后新用户默认需要邀请码。

### 方式二：本地开发

#### 后端

```bash
cargo run --bin api
```

默认后端地址：`http://127.0.0.1:8000`

交互式 API 文档地址：`http://127.0.0.1:8000/docs/`，生成的 OpenAPI JSON 地址：`http://127.0.0.1:8000/openapi.json`。

默认启动会输出 HTTP 请求日志，包含 method、path、status 和 latency。可通过 `RUST_LOG` 调整过滤级别，例如：

```bash
RUST_LOG=error cargo run --bin api
```

另开一个终端可启动 Rust worker shadow 进程：

```bash
cargo run -p ps-cli -- worker shadow --interval-seconds 300
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
| `--workers`, `-w` | `32` | 兼容旧命令的并发参数；当前 Rust live runner 串行执行 |
| `--issue-batch` | 同 `--workers` | CNKI issue 批大小 |
| `--timeout` | `20` | 上游请求超时秒数 |
| `--resume` / `--no-resume` | `--resume` | 跳过已完成期刊或年份 |
| `--update` / `--no-update` | `--no-update` | 增量更新并生成 `data/push_state/*.changes.json` |
| `--notify` | `false` | 更新后调用 `notify` |
| `--notify-dry-run` | `false` | `--notify` handoff 使用 dry-run |

`SEMANTIC_SCHOLAR_API_KEY_POOL`、`PROXY_POOL`、`OPENALEX_API_KEY_POOL` 和 `CROSSREF_MAILTO_POOL` 可通过环境变量或管理员运行时配置表提供，供 Rust 服务和调度命令读取。

### 2. API 服务

```bash
cargo run --bin api
```

API 启动后会提供：

- `/api/*`：业务接口
- `/docs/`：Swagger UI 交互式文档
- `/openapi.json`：由 Rust 注解和 DTO schema 编译期生成的 OpenAPI JSON

终端默认显示请求日志；设置 `RUST_LOG=error` 可只显示 error 级日志。

环境变量：

- `API_HOST`：监听地址，默认 `127.0.0.1`
- `API_PORT`：监听端口，默认 `8000`
- `PAPER_SCANNER_PROJECT_ROOT`：项目根目录，默认当前目录；Docker 中为 `/app`
- `API_CORS_ALLOWED_ORIGINS`：逗号分隔的跨源浏览器 Origin 白名单，默认空
- `AUTH_COOKIE_SECURE`：显式控制 `ps_session` Cookie 的 `Secure` 标记；未设置时按请求 scheme 推断

外部元数据服务运行配置可通过管理员后台写入 `data/auth.sqlite` 的 `runtime_settings` 表。当前受管理的配置项为：

| 配置项 | 说明 |
| --- | --- |
| `OPENALEX_API_KEY_POOL` | OpenAlex API key 池；scholarly 索引需要 |
| `SEMANTIC_SCHOLAR_API_KEY_POOL` | Semantic Scholar API key 池；scholarly 索引需要 |
| `CROSSREF_MAILTO_POOL` | Crossref 联系邮箱池，建议生产环境配置 |
| `PROXY_POOL` | scholarly 与 CNKI 请求代理池 |

API、索引器和调度任务启动时会读取 `runtime_settings` 并覆盖同名进程环境变量；如果数据库没有对应值，则使用宿主或容器环境变量。Docker Compose 同时传入宿主环境变量和挂载 `./data:/app/data`，因此可以复用现有 `data/auth.sqlite` 中的运行时配置。

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

全局运行时默认值通过环境变量提供，推荐使用新的 OpenAI 兼容命名：

| 环境变量 | 默认值 | 说明 |
| --- | --- | --- |
| `NOTIFY_AI_BASE_URL` | `https://api.siliconflow.cn/v1` | 默认 OpenAI 兼容基地址 |
| `NOTIFY_AI_API_KEY` | 空 | 默认 API Key |
| `NOTIFY_AI_MODEL` | `deepseek-ai/DeepSeek-V3` | 默认模型名 |
| `NOTIFY_AI_SYSTEM_PROMPT` | 空 | 默认系统提示词 |
| `NOTIFY_MAX_CANDIDATES` | `120` | 单次送入模型的候选上限 |
| `NOTIFY_TEMPERATURE` | `0.2` | 模型温度 |
| `NOTIFY_PUSHPLUS_CHANNEL` | `wechat` | PushPlus 默认渠道 |
| `NOTIFY_PUSHPLUS_TEMPLATE` | `markdown` | PushPlus 默认模板 |
| `NOTIFY_PUSHPLUS_TOPIC` | 空 | PushPlus 默认 topic |
| `NOTIFY_PUSHPLUS_OPTION` | 空 | PushPlus 默认 option |

通知链路现在只识别上述 `NOTIFY_AI_*` 变量，不再兼容旧的 OpenAI / SiliconFlow 别名。

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

这几条链路都依赖 `*.changes.json` 文件。

## 部署说明

根目录 `docker-compose.yml` 使用三个服务：

- `api`：Rust API 后端，暴露 `8000`
- `worker`：Rust worker shadow 进程，复用后端镜像并挂载同一个 `data` 目录
- `app`：前端，暴露 `3000`

前端在 Docker 构建阶段使用 `INTERNAL_API_URL` 将 `/api/*` 重写到后端；`app/Dockerfile` 默认为 `http://api:8000`。根 Compose 文件里没有显式设置这个变量，是因为 `app/Dockerfile` 已提供该默认值。

当前主前端登录流程只使用后端 `/api/auth/*`。仓库已移除旧的前端令牌认证工具与 `config` 挂载；根 Compose 现在只依赖 `data` 卷。Rust 服务读取现有 `data/index/*.sqlite`、`data/auth.sqlite` 与推送状态文件。

## 详细文档

- [API 文档](docs/api.md)
- [数据库说明](docs/database.md)
- [开发指南](docs/development.md)
- [Docker 部署](docs/docker.md)
- [通知与追踪推送](docs/notify.md)
- [Crossref / OpenAlex / Semantic Scholar 集成](docs/scholarly_api.md)
- [CNKI overseas 集成](docs/cnki_oversea_api.md)
- [前端说明](app/README.md)
- [MCP Server](mcp/README.md)
