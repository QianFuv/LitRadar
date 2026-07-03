# Paper Scanner

Paper Scanner 是一个面向学术期刊的全栈检索与订阅平台。它负责从 Crossref、OpenAlex、Semantic Scholar 与 CNKI overseas 抓取期刊和文章元数据，构建 SQLite 检索库，并提供 Web 界面、收藏夹、追踪推送、每周更新、公告与后台管理能力。

当前 Docker 后端默认运行 Rust 服务。T13 完成前，Python 后端命令仍保留为回滚和 live 索引维护路径：

- `ps-api`：启动兼容现有 `/api/*` 契约的 Rust API 后端
- `ps-cli worker shadow`：启动 Rust worker sidecar，周期性加载定时任务并保持服务运行
- `ps-cli notify dry-run|shadow`：演练或 shadow 比对 PushPlus 通知链路
- `ps-cli push dry-run|shadow`：演练或 shadow 比对追踪文件夹写入链路
- `ps-cli index fixture`：运行 Rust 索引 fixture/parity 命令
- `uv run index`、`uv run api`、`uv run notify`、`uv run push`：T13 前的 Python 回滚命令

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
| 索引/抓取 | Rust workspace crates、SQLite FTS5；Python live 索引命令在 T13 前保留为回滚路径 |
| AI 与推送 | OpenAI 兼容服务、PushPlus |
| 调度 | Rust worker/CLI；Python APScheduler 在 T13 前保留为回滚路径 |
| 开发工具 | Cargo、uv、Ruff、mypy、pnpm |

## 仓库结构

```text
.
├── app/                     前端项目
├── crates/                  Rust 后端 workspace
├── docs/                    详细文档
├── paper_scanner/           Python 回滚后端、索引、推送与公共模块
├── data/
│   ├── meta/                期刊 CSV 元数据源
│   ├── index/               生成后的 SQLite 检索库
│   ├── push_state/          通知与每周更新状态、变更清单
│   ├── folder_push_state/   追踪文件夹推送状态
│   └── auth.sqlite          用户、收藏、通知、管理员数据
├── libs/                    SQLite simple tokenizer 预编译扩展
├── docker-compose.yml       根 Docker Compose 编排
├── Dockerfile               后端镜像构建
└── pyproject.toml           Python 项目配置
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

   如果 `data/index/*.sqlite` 已存在，可直接使用现有索引库。T13 完成前，live 索引维护仍使用宿主机 Python 回滚命令；Rust Docker 镜像不再包含 `uv` 或 Python 后端运行环境。

   ```bash
   uv sync --dev
   uv run index --file utd24.csv
   ```

4. 访问服务

   - 前端：`http://localhost:3000`
   - 后端 API：`http://localhost:8000/api`

5. 注册第一个用户

   第一个注册用户不需要邀请码，并会自动成为管理员。之后新用户默认需要邀请码。

### 方式二：本地开发

#### 后端

```bash
cargo run -p ps-api
```

默认后端地址：`http://127.0.0.1:8000`

另开一个终端可启动 Rust worker shadow 进程：

```bash
cargo run -p ps-cli -- worker shadow --interval-seconds 300
```

T13 完成前，如果需要回滚到 Python 后端或执行 live 索引：

```bash
uv sync --dev
uv run index --file utd24.csv
uv run api
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
cargo run -p ps-cli -- index fixture --csv tests/fixtures/contracts/scholarly/journals.csv --fixture tests/fixtures/contracts/scholarly/openalex_fallback_fixture.json --output-db data/index/scholarly-fixture.sqlite
cargo run -p ps-cli -- index fixture --source cnki --csv tests/fixtures/contracts/cnki/journals.csv --fixture tests/fixtures/contracts/cnki/fixture.json --output-db data/index/cnki-fixture.sqlite
```

Rust fixture/parity 索引参数：

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `--csv` | 必填 | CSV 元数据源路径 |
| `--fixture` | 必填 | recorded fixture 目录或文件 |
| `--output-db` | 必填 | 输出 SQLite 索引库 |
| `--source` | `scholarly` | 可选 `scholarly` 或 `cnki` |
| `--manifest` | 空 | 输出变更清单 |
| `--resume` | `false` | CNKI fixture 索引恢复模式 |
| `--update` | `false` | CNKI fixture 增量模式 |
| `--issue-batch-size` | `10` | CNKI fixture issue 批大小 |

英文 scholarly 路径会对 Crossref、OpenAlex、Semantic Scholar 分别限流。`OPENALEX_API_KEY_POOL` 与 `SEMANTIC_SCHOLAR_API_KEY_POOL` 是 scholarly 索引的必需配置；`CROSSREF_MAILTO_POOL` 建议生产环境配置为可联系邮箱；`PROXY_POOL` 可为 scholarly 与 CNKI 请求提供代理池。Semantic Scholar 按官方 introductory limit 保守处理为全局 1 RPS，并通过 `/graph/v1/paper/batch` 每次最多请求 500 个 DOI。

T13 完成前，live 索引和回滚仍可使用 Python 命令：

```bash
uv run index --file utd24.csv
uv run index --workers 32 --processes 2
uv run index --update --notify
uv run index --update --notify --notify-dry-run
```

### 2. API 服务

```bash
cargo run -p ps-api
```

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

API、索引器和调度任务启动时会读取 `runtime_settings` 并覆盖同名进程环境变量；如果数据库没有对应值，则使用宿主或容器环境变量。T12 Docker Compose 同时传入宿主环境变量和挂载 `./data:/app/data`，因此可以复用现有 `data/auth.sqlite` 中的运行时配置。

### 3. PushPlus 通知推送

```bash
cargo run -p ps-cli -- notify dry-run --auth-db data/auth.sqlite --index-db data/index/utd24.sqlite --db utd24.sqlite
cargo run -p ps-cli -- notify shadow --auth-db data/auth.sqlite --index-db data/index/utd24.sqlite --db utd24.sqlite --changes-file data/push_state/utd24.changes.json
```

该命令只处理 `delivery_method = "pushplus"` 的用户。

T13 完成前，如需回滚到 Python live 发送路径：

```bash
uv run notify --db utd24.sqlite
uv run notify --db utd24.sqlite --changes-file data/push_state/utd24.changes.json
uv run notify --db utd24.sqlite --dry-run
```

### 4. 追踪文件夹推送

```bash
cargo run -p ps-cli -- push dry-run --auth-db data/auth.sqlite --index-db data/index/utd24.sqlite --db utd24.sqlite
cargo run -p ps-cli -- push shadow --auth-db data/auth.sqlite --index-db data/index/utd24.sqlite --db utd24.sqlite --changes-file data/push_state/utd24.changes.json
```

该命令只处理 `delivery_method = "folder"` 且已配置追踪文件夹的用户。

T13 完成前，如需回滚到 Python live 写入路径：

```bash
uv run push --db utd24.sqlite
uv run push --db utd24.sqlite --changes-file data/push_state/utd24.changes.json
uv run push --db utd24.sqlite --dry-run
```

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
- `data/push_state/<db>.json`：PushPlus 通知流水状态
- `data/folder_push_state/<db>.json`：追踪文件夹推送流水状态

说明：

- `/api/weekly-updates`
- `/api/tracking/push-weekly`
- `ps-cli notify shadow --changes-file ...`
- `ps-cli push shadow --changes-file ...`

这几条链路都依赖 `*.changes.json` 文件。

## 部署说明

根目录 `docker-compose.yml` 使用三个服务：

- `api`：Rust API 后端，暴露 `8000`
- `worker`：Rust worker shadow 进程，复用后端镜像并挂载同一个 `data` 目录
- `app`：前端，暴露 `3000`

前端在 Docker 构建阶段使用 `INTERNAL_API_URL` 将 `/api/*` 重写到后端；`app/Dockerfile` 默认为 `http://api:8000`。根 Compose 文件里没有显式设置这个变量，是因为 `app/Dockerfile` 已提供该默认值。

当前主前端登录流程只使用后端 `/api/auth/*`。仓库已移除旧的前端令牌认证工具与 `config` 挂载；根 Compose 现在只依赖 `data` 卷。T12 不修改 SQLite schema 或状态文件格式，因此 Rust 服务和 Python 回滚命令读取同一套 `data/index/*.sqlite`、`data/auth.sqlite` 与推送状态文件。

T13 完成前，如需回滚到 Python 后端：

1. 停止 Docker Rust 后端服务：`docker compose stop api worker`
2. 在宿主机启动 Python API：`uv run api`
3. 如需继续使用前端容器，使用 `INTERNAL_API_URL=http://host.docker.internal:8000` 重新构建前端镜像；或者停止 `app` 容器后在本地运行前端开发服务。

回滚不需要数据库迁移；不要删除 `data/`。

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
