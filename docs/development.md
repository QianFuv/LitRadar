# 开发指南

本文档从当前代码结构出发，说明 Paper Scanner 的主要模块、数据流、运行行为与开发注意事项。

## 一、整体架构

```text
CSV 元数据
  -> 索引器（scripts/index）
  -> data/index/*.sqlite
  -> FastAPI（scripts/api）
  -> Next.js 前端（app）

增量更新
  -> data/push_state/*.changes.json
  -> 每周更新页面
  -> notify（PushPlus）
  -> push（追踪文件夹）
```

## 二、模块划分

### 1. `scripts/index/`

负责离线抓取和增量更新。

关键职责：

- 读取 `data/meta/*.csv`
- BrowZine 与维普抓取
- 建立或更新 `data/index/*.sqlite`
- 维护 `article_listing` 与 `article_search`
- 在 `--update` 模式下生成变更清单
- 在 `--notify` 模式下联动调用 `notify`

关键文件：

- `scripts/index/main.py`
- `scripts/index/fetcher.py`
- `scripts/index/changes.py`
- `scripts/index/db/schema.py`
- `scripts/index/db/operations.py`

### 2. `scripts/api/`

负责 FastAPI 服务、认证数据库、调度器与 REST 路由。

关键职责：

- 对外提供检索、认证、收藏、追踪、后台管理 API
- 初始化 `data/auth.sqlite`
- 启动 APScheduler 背景调度器
- 为 `/api/articles*` 与 `/api/meta*` 添加缓存头

关键文件：

- `scripts/api/app.py`
- `scripts/api/main.py`
- `scripts/api/routes/*`
- `scripts/api/auth_db.py`
- `scripts/api/scheduler.py`

### 3. `scripts/notify/`

负责 PushPlus 通知链路。

关键职责：

- 读取变更清单或增量快照
- 加载数据库中的订阅用户
- 调用 OpenAI 兼容模型做候选筛选
- 构造 Markdown 推送正文
- 更新 `data/push_state/*.json`

### 4. `scripts/push/`

负责把新增文章写入追踪文件夹。

它与 `notify` 共用很多候选选择逻辑，但最终写入的是 `favorites` 表，而不是 PushPlus。

### 5. `app/`

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
2. 根据 `library` 判断数据源：
   - `-1` -> 维普
   - 其他 -> BrowZine
3. 抓取 journal / issue / article
4. 写入 `journals`、`issues`、`articles`
5. 刷新 `article_listing`
6. 更新 FTS5 表 `article_search`
7. 在增量模式下生成 `*.changes.json`

### 2. 检索流

1. 前端通过 `app/lib/api.ts` 发起请求
2. API 通过 `db` 参数解析目标索引库
3. `/api/articles` 优先使用 `article_listing`
4. 若需要全文检索，则联动 `article_search`
5. 返回分页结果给前端

### 3. 每周更新流

1. 读取 `data/push_state/*.changes.json`
2. 聚合变更清单中的新增文章，并根据清单时间戳生成响应窗口
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

访问受保护接口时使用 Bearer 令牌。

前端通过 `app/lib/auth-context.tsx` 与后端 `/api/auth/*` 维护登录态，不再包含额外的本地令牌认证配置。

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
uv run index --file utd24.csv
uv run api
```

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
uv run ruff check scripts
uv run ruff format scripts
uv run mypy scripts
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

不是。当前页面只依赖后端账号体系与 Bearer 令牌流程。

### 3. 认为通知配置只支持 SiliconFlow

不是。当前代码的真实能力是：

- 支持任意 OpenAI 兼容接口
- 环境变量统一使用 `NOTIFY_AI_*`，不再兼容旧的 OpenAI / SiliconFlow 别名

### 4. 认为管理员定时任务只记录配置不执行

不是。它们会在 API 进程内被 APScheduler 真实调度执行。
