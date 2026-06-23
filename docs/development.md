# 开发指南

本文档从当前代码结构出发，说明 Paper Scanner 的主要模块、数据流、运行行为与开发注意事项。

## 一、整体架构

```text
CSV 元数据
  -> 索引器（paper_scanner/index）
  -> data/index/*.sqlite
  -> FastAPI（paper_scanner/api）
  -> Next.js 前端（app）

增量更新
  -> data/push_state/*.changes.json
  -> 每周更新页面
  -> notify（PushPlus）
  -> push（追踪文件夹）
```

## 二、模块划分

### 1. `paper_scanner/index/`

负责离线抓取和增量更新。

关键职责：

- 读取 `data/meta/*.csv`
- Crossref / OpenAlex / Semantic Scholar 与 CNKI overseas 抓取
- 建立或更新 `data/index/*.sqlite`
- 维护 `article_listing` 与 `article_search`
- 在 `--update` 模式下生成变更清单
- 在 `--notify` 模式下联动调用 `notify`

关键文件：

- `paper_scanner/index/main.py`
- `paper_scanner/index/fetcher.py`
- `paper_scanner/index/changes.py`
- `paper_scanner/index/db/schema.py`
- `paper_scanner/index/db/operations.py`

### 2. `paper_scanner/sources/`

负责外部元数据源客户端。

关键职责：

- `cnki/`：CNKI overseas 期刊、期次与文章元数据抓取
- `scholarly/`：Crossref / OpenAlex / Semantic Scholar 元数据抓取
- 统一使用共享请求池、代理池和运行配置

关键文件：

- `paper_scanner/sources/cnki/client.py`
- `paper_scanner/sources/scholarly/client.py`

### 3. `paper_scanner/api/`

负责 FastAPI 服务、认证数据库、调度器与 REST 路由。

关键职责：

- 对外提供检索、认证、收藏、追踪、后台管理 API
- 初始化 `data/auth.sqlite`
- 启动 APScheduler 背景调度器
- 为 `/api/articles*` 与 `/api/meta*` 添加缓存头

关键文件：

- `paper_scanner/api/app.py`
- `paper_scanner/api/main.py`
- `paper_scanner/api/routes/*`
- `paper_scanner/api/auth_db.py`
- `paper_scanner/api/scheduler.py`

### 4. `paper_scanner/notify/`

负责 PushPlus 通知链路。

关键职责：

- 读取变更清单或增量快照
- 加载数据库中的订阅用户
- 调用 OpenAI 兼容模型做候选筛选
- 构造 Markdown 推送正文
- 更新 `data/push_state/*.json`

### 5. `paper_scanner/push/`

负责把新增文章写入追踪文件夹。

它与 `notify` 共用很多候选选择逻辑，但最终写入的是 `favorites` 表，而不是 PushPlus。

### 6. `app/`

负责前端页面与用户交互。

当前真实页面包括：

- 登录注册
- 搜索首页
- 每周更新
- 收藏夹
- 文献追踪
- 设置页
- 管理后台
- 首页公告

## 三、真实数据流

### 1. 索引流

1. 读取 `data/meta/*.csv`
2. 根据 `source` 判断数据源：
   - `scholarly` -> Crossref / OpenAlex / Semantic Scholar
   - `cnki` -> CNKI overseas
3. 抓取 journal / issue / article
4. 写入 `journals`、`issues`、`articles`
5. 刷新 `article_listing`
6. 更新 FTS5 表 `article_search`
7. 在增量模式下生成 `*.changes.json`

增量模式会重新拉取期刊的年份与 issue 列表，抓取本地还没有文章的 issue，并额外重扫最新一个已有文章的 issue，用于补充已发布 issue 后续追加的文章。

### 2. 检索流

1. 前端通过 `app/lib/api.ts` 发起请求
2. API 通过 `db` 参数解析目标索引库
3. `/api/articles` 优先使用 `article_listing`
4. 若需要全文检索，则联动 `article_search`
5. 返回分页结果给前端

### 3. 每周更新流

1. 读取 `data/push_state/*.changes.json`
2. 聚合变更清单中 `notifiable_article_ids` 的新增文章，并根据清单时间戳生成响应窗口
3. 回到各索引库按 `article_id` 取回文章详情
4. 按数据库和期刊组织成 `/api/weekly-updates` 响应

### 4. 通知与追踪流

1. 读取变更清单或快照差异
2. 生成候选文章
3. 按订阅用户加载通知配置
4. 使用 OpenAI 兼容模型做筛选；配置不可用时跳过对应订阅用户
5. 进入两条终端链路之一：
   - PushPlus 消息发送
   - 写入追踪文件夹

## 四、当前 API 路由面

当前后端实际注册的路由模块包括：

- `health`
- `meta`
- `journals`
- `issues`
- `articles`
- `weekly`
- `announcements`
- `auth`
- `favorites`
- `tracking`
- `admin`

旧文档里只写到 `health/meta/journals/issues/articles/weekly` 的说法已经过时。

## 五、鉴权与权限模型

### 用户鉴权

当前主流程使用后端账号系统：

- `/api/auth/register`
- `/api/auth/login`
- `/api/auth/me`
- `/api/auth/tokens`

浏览器登录态使用后端设置的 `HttpOnly` `ps_session` Cookie，前端不在 `localStorage` 或 `sessionStorage` 保存登录令牌。刷新页面时，`app/lib/auth-context.tsx` 通过 `/api/auth/me` 携带 Cookie 恢复当前用户。

设置页创建的访问令牌只面向外部脚本/API 客户端，调用受保护接口时使用 `Authorization: Bearer <access_token>`。不要把令牌放入 URL 查询参数。

### 管理员权限

首个注册用户会自动成为管理员，管理员可访问：

- 用户管理
- 邀码管理
- 系统统计
- 定时任务管理
- 公告管理

## 六、调度器行为

API 启动时会在 `lifespan` 中执行：

1. `init_auth_db()`
2. `start_scheduler()`

调度器从 `scheduled_tasks` 表中加载启用任务，并按 cron 表达式执行 shell 命令。

需要注意：

- 执行方式是 `subprocess.run(..., shell=True)`
- 调度器与 API 服务运行在同一进程空间
- 修改任务后会调用 `reload_scheduler()` 重新装载

## 七、目录与状态文件

### 运行时重点目录

| 路径 | 用途 |
| --- | --- |
| `data/meta/` | 期刊 CSV 输入 |
| `data/index/` | 索引数据库输出 |
| `data/auth.sqlite` | 用户与业务数据库 |
| `data/push_state/` | 通知状态与变更清单 |
| `data/folder_push_state/` | 追踪文件夹推送状态 |
| `libs/simple-*` | SQLite 中文分词扩展 |

## 八、本地开发建议

### Python 侧

```bash
uv sync --dev
uv run index --file utd24.csv --workers 32 --processes 3
uv run api
```

外部元数据服务配置由管理后台写入 `data/auth.sqlite` 的运行配置表。Docker 或宿主进程环境变量只作为启动输入；数据库已有值时会覆盖同名环境变量。

### 前端

```bash
cd app
corepack enable pnpm
pnpm install
pnpm dev
```

## 九、修改代码后的检查

根据仓库约束，修改 Python 代码后应至少执行：

```bash
uv run ruff check paper_scanner
uv run ruff format paper_scanner
uv run mypy paper_scanner
```

如果修改了前端代码，建议额外执行：

```bash
cd app
pnpm lint
```

## 十、常见误区

### 1. 把每周更新理解为“按数据库日期扫描”

不是。当前实现依赖：

- `data/push_state/*.changes.json`

没有变更清单，就不会有每周更新、通知或追踪推送。

### 2. 认为前端还保留独立的本地令牌认证配置

不是。当前页面只依赖后端账号体系与 `ps_session` Cookie 登录态；Bearer 访问令牌只用于外部脚本/API 客户端。

### 3. 认为通知配置只支持 SiliconFlow

不是。当前代码的真实能力是：

- 支持任意 OpenAI 兼容接口
- 环境变量统一使用 `NOTIFY_AI_*`，不再兼容旧的 OpenAI / SiliconFlow 别名

### 4. 认为管理员定时任务只记录配置不执行

不是。它们会在 API 进程内被 APScheduler 真实调度执行。
