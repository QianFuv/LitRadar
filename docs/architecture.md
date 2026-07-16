# 系统架构

本文档说明 LitRadar 当前的组件边界、进程模型、数据流和持久化结构。命令参数见 [CLI 参考](reference/cli.md)，具体部署步骤见 [Docker 部署](operations/docker.md)。

## 总览

```text
browser
   |
   v
litradar serve  (one long-running process)
   |-- HTTP component: static Web / REST / Swagger / OpenAPI / MCP
   |-- embedded scheduler: persistent cursor, claims and heartbeats
   |       |
   |       +-- current executable -> litradar index  (transient child)
   |       +-- current executable -> litradar notify (transient child)
   |       +-- current executable -> litradar push   (transient child)
   |
   +-- data/auth.sqlite
   +-- data/index/*.sqlite
   +-- data/push_state/*.json
   +-- data/folder_push_state/*.json

/usr/share/litradar/meta  (immutable image bundle)
              |
              +-- startup preparation -> data/meta/*.csv  (persistent copies)
                                           |
                                           +-- litradar index -> data/index/*.sqlite
                                                               -> *.changes.json -> litradar notify / litradar push
```

系统没有 Python 运行时路径。Node.js 只在镜像构建阶段把 `app/` 导出为静态资源；生产运行层不包含 Node.js。Rust workspace 只发布 `litradar` 一个可执行文件，所有能力都通过它的七个子命令进入。

## 运行进程

| 入口                 | 生命周期   | 职责                                                        |
| -------------------- | ---------- | ----------------------------------------------------------- |
| `litradar serve`     | 唯一常驻   | HTTP 服务与内嵌调度；统一负责准备、信号、关闭和组件失败传播 |
| `litradar index`     | 按需或调度 | 读取期刊 CSV、请求上游、写索引库和变更清单                  |
| `litradar notify`    | 按需或调度 | AI 选择后发送 PushPlus                                      |
| `litradar push`      | 按需或调度 | AI 选择后写入用户追踪文件夹                                 |
| `litradar scheduler` | 按需       | 校验任务或立即执行一个已保存的类型化任务                    |
| `litradar admin`     | 按需、本机 | 初始化管理员、维护密文、备份和离线恢复                      |
| `litradar openapi`   | 按需       | 把当前 REST schema 输出到 stdout 或文件                     |

Docker Compose 只运行一个名为 `litradar` 的容器。`app/` 是镜像构建输入，不是运行服务。调度和索引需要进程隔离时，父进程通过当前可执行路径再次启动 `litradar` 并传入规范子命令；镜像中不存在按功能拆分的其他可执行文件。

SIGINT 或 SIGTERM 会同时通知 HTTP 与调度组件。若计划任务子进程正在运行，调度器先终止并等待该子进程，将运行状态持久化为 `cancelled`，且不再启动剩余步骤。HTTP、心跳或调度组件意外退出时，组合根会关闭另一组件并以非零状态退出，避免半可用进程继续运行。

## Rust workspace

| Crate                | 责任                                                          |
| -------------------- | ------------------------------------------------------------- |
| `litradar`           | 唯一二进制、七个子命令分发、服务组合、信号和组件生命周期      |
| `litradar-api`       | 可准备和注入关闭信号的 Axum 库、OpenAPI、MCP 与请求异步边界   |
| `litradar-auth`      | 密码、会话、访问令牌和认证服务                                |
| `litradar-cli`       | `admin`、索引、投递和手动调度子命令的库级参数适配与编排       |
| `litradar-domain`    | 跨 crate 的领域结构和响应类型                                 |
| `litradar-index`     | 索引 schema、转换、写入、统计和变更清单                       |
| `litradar-recommend` | 候选排序、AI 配置解析、消息内容和投递状态                     |
| `litradar-sources`   | Crossref、OpenAlex、Semantic Scholar、CNKI 和浙江图书馆客户端 |
| `litradar-storage`   | SQLite 迁移、查询、业务存储、密文和备份恢复                   |
| `litradar-worker`    | 内嵌调度、认领、子进程取消、AI/PushPlus 传输和投递编排        |

依赖方向以领域结构和存储接口为中心；只有 `crates/litradar` 拥有进程入口，其余 crate 都是库。

## 持久化边界

### 元数据输入

`data/meta/*.csv` 定义需要索引的期刊。核心列包括：

| 列          | 含义                  |
| ----------- | --------------------- |
| `source`    | `scholarly` 或 `cnki` |
| `title`     | 期刊标题              |
| `issn`      | 首选 ISSN             |
| `id`        | 稳定的上游/项目标识   |
| `area`      | 项目领域标签          |
| `all_issns` | 可选 ISSN 候选列表    |

### 受管目录生命周期

发布镜像把不可变官方源文件和 `bundle-manifest.json` 放在 `/usr/share/litradar/meta`，通过 `LITRADAR_BUNDLED_META_DIR` 把该打包位置交给应用。运行时只在持久的 `<project-root>/data/meta` 中创建或替换副本；不会修改镜像 bundle。清单使用格式 `litradar-meta-bundle`、正整数版本以及每个当前文件和已知官方旧版的规范 SHA-256。已应用版本和 hash 记录在 `data/auth.sqlite.managed_meta_catalogs`，因此认证库迁移总是在准备之前完成。

每个清单文件独立分类：

| 持久目录状态                                           | 准备行为                                     |
| ------------------------------------------------------ | -------------------------------------------- |
| 文件缺失                                               | 原子写入当前官方文件并记录受管状态           |
| 文件等于当前官方 hash，但没有当前状态                  | 接管并记录状态，不重写文件                   |
| 文件等于清单中的已知旧版 hash，或仍等于上次已应用 hash | 原子升级到当前官方文件                       |
| 同名普通文件具有其他内容，包括非 UTF-8 内容            | 视为用户自定义，保留文件和旧状态，并报告冲突 |
| 文件名不在当前清单中                                   | 完全不读取、不替换、不删除                   |

符号链接、目录或特殊文件占用受管目标路径时会作为不安全布局失败。一次准备在 `BEGIN IMMEDIATE` 事务中更新状态，并为文件替换保留回滚副本；文件或状态写入失败会恢复本次已经替换的文件。若认证库记录的 bundle 版本高于当前镜像版本，应用会在修改目录前拒绝降级启动。这使镜像回滚成为显式数据兼容决策，不能靠覆盖文件绕过。

从清单退役或改名的旧文件不会自动删除，相关状态行也会保留。运维人员应先创建并验证包含完整 Meta 树的 v2 备份，再确认没有索引任务引用旧文件，最后手工清理明确选中的路径；未知文件不能批量删除。

准备入口和后续顺序保持一致：

- `litradar serve`：迁移认证库和现有索引库，准备 Meta，验证密钥并加载运行设置，构建 HTTP 服务，然后启动内嵌调度。
- 普通 `litradar index`：迁移数据库，准备 Meta，验证密钥并加载 scholarly 设置，然后进入下述现有 Meta 期刊预检和索引流程。
- 多进程索引的内部 worker 请求不重复准备；它使用父进程已经准备和预检的持久目录。
- 未设置 `LITRADAR_BUNDLED_META_DIR` 的本地构建不执行受管准备，缺失或空的 `data/meta` 仍沿用原有无输入行为。

准备报告以结构化 JSON 写入 stderr；索引命令的 stdout JSON 契约不变。该步骤只管理打包清单声明的文件，不替代下面的逐 CSV 身份和数据库投影预检。

### Meta 目录预检边界

`litradar index` 对当前选中的每个 CSV 独立执行以下门禁：

```text
read selected CSV
  -> validate supported sources and per-file stable-ID uniqueness
  -> acquire/refresh index-run lease and start heartbeat
  -> immediate transaction: synchronize local catalog -> verify every row
  -> commit
  -> local journal processing or process workers -> upstream requests
```

静态身份校验发生在数据库和父运行创建之前。目录同步和复验发生在同一立即事务内，并由当前 `index_run_lease` 所有者隔离；任一行失败都会回滚全部目录变更，正常失败收尾会记录父运行、释放本方租约且不启动 worker。多进程索引、`--update` 和内嵌调度启动的索引都汇合到同一 live CSV 编排，因此没有绕过门禁的独立路径。

字段所有权保持窄边界：

| 表/字段 | 预检行为 |
| ------- | -------- |
| 缺失的 `journals` 行 | 从 CSV 身份建立中性壳；provider 可用性、文章、排名等字段为 `NULL` |
| 已有的 `journals` 行 | 不更新，保留数据源已经解析的全部字段 |
| `journal_meta.source_csv/area/csv_title/csv_issn/csv_library` | 从当前 CSV 插入或按差异更新 |
| `journal_meta.resolved_*` | 插入时为 `NULL`；已有值原样保留 |

该门禁不调用外部 provider，不增加 API 请求、线程或进程，也不尝试模糊修正 CSV，因此是 rate-limit neutral。它保证本地 Meta 目录无歧义且数据库投影已经同步，不能保证后续 Crossref、OpenAlex、Semantic Scholar 或 CNKI 请求一定成功。经过审查的身份修正属于 `data/meta/*.csv` 的源数据变更。

### 索引数据库

每个 CSV 对应 `data/index/<csv_stem>.sqlite`。索引库包含期刊、期次、文章、FTS5、物化筛选表和索引运行统计。详见[数据库参考](reference/database.md)。

### 认证与业务数据库

`data/auth.sqlite` 保存：

- 用户、会话和访问令牌
- CNKI 用户会话
- 收藏夹和收藏文章
- 通知/追踪设置
- 全局运行配置
- 类型化定时任务、运行槽和心跳
- 系统公告

该库不保存 32 字节部署密钥；受保护的集成凭据以认证密文写入数据库，密钥通过文件单独提供。

### 外部状态文件

| 路径                                | 所有者                                                       |
| ----------------------------------- | ------------------------------------------------------------ |
| `data/push_state/<db>.changes.json` | `litradar index --update` 生成；每周更新、通知和追踪共同读取 |
| `data/push_state/<db>.json`         | `litradar notify` 和手动 PushPlus 投递状态                   |
| `data/folder_push_state/<db>.json`  | `litradar push` 的追踪文件夹投递状态                         |

变更清单是新文章分发的输入，不是可从文章日期实时重建的视图。读取方只以必填的 `db_name` 识别目标数据库，不使用保存的文件系统路径作为身份回退。

## 主要数据流

### Scholarly 索引

1. `litradar index` 读取 `source=scholarly` 的 CSV 行。
2. Crossref 按 ISSN 提供文章主列表。
3. Crossref 对所有 ISSN 返回 404 时，OpenAlex 解析 source 并作为文章列表 fallback。
4. OpenAlex 按 DOI 增强元数据；Semantic Scholar 增强 OA、PDF 和缺失摘要。
5. `litradar-index` 写入关系表、`article_listing` 和 `article_search`。
6. `--update` 生成变更清单。

具体请求和字段优先级见 [Scholarly 数据源](reference/sources/scholarly.md)。

### CNKI 索引和全文

CNKI 元数据索引使用公开 overseas 页面和接口；按用户全文获取使用独立的浙江图书馆会话。索引器不把权限控制链接当作可直接访问的全文。详见 [CNKI 数据源](reference/sources/cnki.md)。

### 检索

1. 浏览器加载 Rust 提供的静态前端，并通过 `app/lib/api/` 同源调用 `/api/*`。
2. API 根据可选 `db` 选择一个 `data/index/*.sqlite`。
3. 列表查询在 `listing_state=ready` 且物化表非空时使用 `article_listing`，否则回退到关系表联查。
4. `q` 使用 `article_search MATCH`。
5. API 把 64 位文章和期刊 ID 序列化为十进制字符串。

### 认证

浏览器登录后使用 `HttpOnly` 的 `litradar_session` Cookie。外部客户端使用用户创建的 Bearer 访问令牌。首个管理员只能通过本机 `litradar admin bootstrap` 创建；公开注册始终要求邀请码。

### 通知和追踪

1. 从变更清单或状态快照差异得到候选文章。
2. 按用户的数据库、关键词和方向偏好缩小候选。
3. 使用用户级 OpenAI 兼容凭据做主备 AI 选择。
4. 根据 `delivery_method` 发送 PushPlus 或写入追踪文件夹。
5. 成功副作用完成后更新 `delivery_dedupe`。

完整行为见[通知与追踪](guides/notifications.md)。

## 配置层次

系统使用多种明确分离的配置来源：

- CLI 参数：`litradar` 子命令的路径、端口、并发和一次运行覆盖
- `runtime_settings`：外部元数据 key 池、CORS、MCP 和 Cookie 策略
- `notification_settings`：每个用户的 AI、PushPlus 和追踪偏好
- 前端构建/开发变量：可选浏览器 API 地址和仅供 `next dev` 使用的内部 Rust rewrite 目标
- 部署密钥文件：只用于认证和解密数据库中的秘密值

来源、默认值和优先级见[运行配置](reference/configuration.md)。

## 数据库迁移

所有正式子命令在业务访问前执行所需的版本化 SQLite 迁移：

1. 解析路径和参数。
2. 检查 `PRAGMA user_version`。
3. 在独立 `BEGIN IMMEDIATE` 事务中逐版本迁移。
4. 在同一事务末尾更新版本。
5. 遇到未来版本或失败立即退出。

`litradar serve` 先完成一次存储迁移、密钥验证和 HTTP 准备，再启动监听器与立即执行的首个调度 tick。普通查询仓库不负责 DDL。

## 同步工作与异步服务

SQLite、PBKDF2、阻塞 HTTP、文件系统和手动推送编排是同步工作。HTTP 组件通过 `ApiState` 的有界阻塞执行器把这些工作送入 Tokio blocking pool，避免在路由或 MCP future 中直接阻塞运行时。内嵌调度 tick 也通过 `spawn_blocking` 运行；按需子命令使用同步作业模型。

HTTP 组件共享的 `blocking executor` 有 8 个 permit。除此之外，手动周推在每个 `litradar serve` 进程内按 `auth.sqlite` 路径设置 1 个 admission slot：同一用户重复启动会复用当前 running job，不同用户竞争同一 storage instance 时立即收到 `503`，因此最多 1 个 manual API job 等待或占用共享 permit。该边界不排队、不持久化，也不是 `cross-process` 锁；独立调用的 `litradar notify`、`litradar push` 或计划任务子进程不受它协调。

## 部署边界

默认 Compose 只运行一个非 root、只读根文件系统且丢弃全部 Linux capabilities 的 `litradar` 容器，并把唯一 HTTP 入口 `127.0.0.1:8000` 发布到宿主机 loopback。公网部署必须增加 TLS 反向代理和共享限流，不能把默认端口直接改为所有网卡。详见 [Docker 部署](operations/docker.md)和[安全说明](operations/security.md)。
