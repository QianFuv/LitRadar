# Paper Scanner

Paper Scanner 是一个面向学术期刊的全栈检索与订阅平台。它负责从 OpenAlex、Crossref、Unpaywall 与 CNKI overseas 抓取期刊和文章元数据，构建 SQLite 检索库，并提供 Web 界面、收藏夹、追踪推送、每周更新、公告与后台管理能力。

当前仓库包含四类核心运行单元：

- `uv run index`：抓取期刊与文章数据，写入 `data/index/*.sqlite`
- `uv run api`：启动 FastAPI 后端
- `uv run notify`：把新增文章筛选后通过 PushPlus 推送给订阅用户
- `uv run push`：把新增文章筛选后写入用户的追踪文件夹

## 主要功能

- 多数据源索引：英文期刊使用 Crossref + OpenAlex + Unpaywall，中文期刊使用 CNKI overseas
- SQLite 检索：文章全文检索基于 FTS5，可选加载 `simple` 中文分词扩展
- 多维筛选：按期刊、领域、年份、开放获取等条件过滤
- 每周更新：基于变更清单聚合最近新增文章
- 用户系统：注册、登录、邀请码、访问令牌、改密
- 收藏与导出：文件夹管理、批量收藏、BibTeX / RIS / EndNote XML 导出
- 文献追踪：将某个文件夹设为追踪文件夹，并按用户偏好自动写入相关文章
- AI 选择：支持 OpenAI 兼容模型配置，不局限于单一服务商
- 管理后台：用户、邀请码、系统统计、定时任务、系统公告
- 首页公告：后台可配置全局公告，前台按优先级展示并支持本地关闭

## 技术栈

| 层级 | 组件 |
| --- | --- |
| 前端 | Next.js 16、React 19、TypeScript、Tailwind CSS 4、Radix UI、TanStack Query |
| 后端 | FastAPI、Uvicorn、aiosqlite |
| 索引/抓取 | httpx、SQLite FTS5 |
| AI 与推送 | OpenAI Python SDK（兼容 OpenAI API 的服务）、PushPlus |
| 调度 | APScheduler |
| 开发工具 | uv、Ruff、mypy、pnpm |

## 仓库结构

```text
.
├── app/                     前端项目
├── docs/                    详细文档
├── scripts/                 后端、索引、推送与公共模块
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

2. 构建并启动服务

   ```bash
   docker compose build
   docker compose up -d
   ```

3. 首次建立索引

   ```bash
   docker compose run --rm api uv run index
   ```

4. 访问服务

   - 前端：`http://localhost:3000`
   - 后端 API：`http://localhost:8000/api`

5. 注册第一个用户

   第一个注册用户不需要邀请码，并会自动成为管理员。之后新用户默认需要邀请码。

### 方式二：本地开发

#### 后端

```bash
uv sync --dev
uv run index --file utd24.csv
uv run api
```

默认后端地址：`http://127.0.0.1:8000`

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
uv run index --file utd24.csv
uv run index --workers 32 --processes 3
uv run index --update --notify
uv run index --update --notify --notify-dry-run
```

常用参数：

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `--file, -f` | 处理 `data/meta/` 下全部 CSV | 指定单个 CSV |
| `--workers, -w` | `32` | 最大并发请求数 |
| `--issue-batch` | `workers * 3` | 每批抓取 issue 数量；传 `0` 时自动计算 |
| `--timeout` | `20` | HTTP 超时秒数 |
| `--processes` | `3` | 期刊级多进程并行数 |
| `--resume / --no-resume` | `--resume` | 是否跳过已完成的期刊/年份 |
| `--update / --no-update` | `--no-update` | 是否增量更新已存在数据库；会抓取新增 issue，并额外重扫最新一个已有文章的 issue |
| `--notify / --no-notify` | `--no-notify` | 更新后自动调用 `notify` |
| `--notify-dry-run` | `false` | 与 `--notify` 配合，仅演练通知不真正推送 |

### 2. API 服务

```bash
uv run api
```

环境变量：

- `API_HOST`：监听地址，默认 `127.0.0.1`

### 3. PushPlus 通知推送

```bash
uv run notify --db utd24.sqlite
uv run notify --db utd24.sqlite --changes-file data/push_state/utd24.changes.json
uv run notify --db utd24.sqlite --dry-run
```

该命令只处理 `delivery_method = "pushplus"` 的用户。

### 4. 追踪文件夹推送

```bash
uv run push --db utd24.sqlite
uv run push --db utd24.sqlite --changes-file data/push_state/utd24.changes.json
uv run push --db utd24.sqlite --dry-run
```

该命令只处理 `delivery_method = "folder"` 且已配置追踪文件夹的用户。

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
- 主要表：`journals`、`journal_meta`、`issues`、`articles`、`article_listing`、`article_search`

### 用户数据库

- 路径：`data/auth.sqlite`
- 主要表：`users`、`access_tokens`、`folders`、`favorites`、`invite_codes`、`notification_settings`、`scheduled_tasks`、`announcements`

### 变更与推送状态

- `data/push_state/<db>.changes.json`：索引增量更新时生成的变更清单
- `data/push_state/<db>.json`：PushPlus 通知流水状态
- `data/folder_push_state/<db>.json`：追踪文件夹推送流水状态

说明：

- `/api/weekly-updates`
- `/api/tracking/push-weekly`
- `uv run notify --changes-file ...`
- `uv run push --changes-file ...`

这几条链路都依赖 `*.changes.json` 文件。

## 部署说明

根目录 `docker-compose.yml` 使用两个服务：

- `api`：后端，暴露 `8000`
- `app`：前端，暴露 `3000`

前端在 Docker 构建阶段使用 `INTERNAL_API_URL` 将 `/api/*` 重写到后端；`app/Dockerfile` 默认为 `http://api:8000`。根 Compose 文件里没有显式设置这个变量，是因为 `app/Dockerfile` 已提供该默认值。

当前主前端登录流程只使用后端 `/api/auth/*`。仓库已移除旧的前端令牌认证工具与 `config` 挂载；根 Compose 现在只依赖 `data` 卷。

## 详细文档

- [API 文档](docs/api.md)
- [数据库说明](docs/database.md)
- [开发指南](docs/development.md)
- [Docker 部署](docs/docker.md)
- [通知与追踪推送](docs/notify.md)
- [OpenAlex / Crossref / Unpaywall 集成](docs/scholarly_api.md)
- [CNKI overseas 集成](docs/cnki_oversea_api.md)
- [前端说明](app/README.md)
