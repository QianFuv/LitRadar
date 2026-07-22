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
   +-- data/index-control/*.sqlite  (disposable provider checkpoints/leases)
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

## 日志流

唯一二进制在分发子命令前从 `data/auth.sqlite.runtime_settings` 只读加载 `log_format` 和 `log_filter`，再安装一个进程级 tracing subscriber。数据库或表尚不存在时使用安全默认值；已存在但非法的值会让进程在业务工作前失败。所有 crate 只产生结构化事件，不各自初始化 subscriber 或写应用日志文件：

```text
process / request / workflow spans
             |
             v
bounded non-blocking queue (4096 lines, lossy)
             |
             v
application stderr (JSON Lines by default)
             |
             +-- local CLI / supervisor
             +-- Docker local driver -> 10 MiB x 5 compressed rotation

browser boundary -> allowlisted console.error object -> local DevTools only
```

HTTP 外层先移除不受信的 `X-Request-Id`，再生成并返回服务器 UUID；请求内的异步和 blocking 工作重新进入同一 span。成功健康检查、静态资源与前端文档不产生逐请求终态事件。调度 span 使用 `run_id` 关联任务；父进程把该值作为经过校验的隐藏内部参数交给规范子进程，应用在公共命令解析前移除参数并记录 `parent_run_id`。它不出现在 help 中，也不是运维配置。子进程 `stderr` 直接继承到同一容器 sink，`stdout` 仍保留给结构化业务结果。

队列饱和时业务线程不等待；正常关闭在排空后直接报告精确丢失行数。浏览器静态客户端没有日志采集端点，只在本地控制台记录不含错误消息和内容的白名单对象。完整字段、配置、保留、隐私和事故流程以[日志运维](operations/logging.md)为准。

## Rust workspace

| Crate                | 责任                                                        |
| -------------------- | ----------------------------------------------------------- |
| `litradar`           | 唯一二进制、七个子命令分发、服务组合、信号和组件生命周期    |
| `litradar-api`       | 可准备和注入关闭信号的 Axum 库、OpenAPI、MCP 与请求异步边界 |
| `litradar-auth`      | 密码、会话、访问令牌和认证服务                              |
| `litradar-cli`       | `admin`、索引、投递和手动调度子命令的库级参数适配与编排     |
| `litradar-domain`    | 规范目录、期刊/期次/文章、在线访问和响应类型                |
| `litradar-provider`  | 可组合 Provider traits、注册表、错误分类和 conformance 检查 |
| `litradar-index`     | 稳定身份、内容 schema、控制状态编排、统一写入和变更清单     |
| `litradar-recommend` | 候选排序、AI 配置解析、消息内容和投递状态                   |
| `litradar-sources`   | 把 Crossref/OpenAlex/S2/CNKI/ZJLib 适配到规范 Provider 能力 |
| `litradar-storage`   | SQLite 迁移、查询、业务存储、密文和备份恢复                 |
| `litradar-worker`    | 内嵌调度、认领、子进程取消、AI/PushPlus 传输和投递编排      |

依赖方向以领域结构和存储接口为中心；只有 `crates/litradar` 拥有进程入口，其余 crate 都是库。

## 持久化边界

### 元数据输入

`data/meta/*.csv` 是 LitRadar 维护的 Provider 无关期刊目录。每行使用不可变 `catalog_id`，并包含规范标题、印刷/电子/全部 ISSN、标题别名、领域和排名字段。目录中没有 `source`、Provider 名称、上游 ID、URL、路由或检查点。

目录格式、规范化和变更规则见[索引与 Provider 契约](reference/index-provider-contract.md)。CSV 文件名 stem 同时决定稳定内容库名；更换 Provider 不改文件名或 `catalog_id`。

### 受管目录生命周期

发布镜像把不可变官方源文件和 `bundle-manifest.json` 固定放在 `/usr/share/litradar/meta`。应用只在该精确 manifest 存在时识别打包运行时，不接受环境变量或 CLI 路径覆盖。运行时只在持久的 `<project-root>/data/meta` 中创建或替换副本；不会修改镜像 bundle。清单使用格式 `litradar-meta-bundle`、正整数版本以及每个当前文件和已知官方旧版的规范 SHA-256。已应用版本和 hash 记录在 `data/auth.sqlite.managed_meta_catalogs`，因此认证库迁移总是在准备之前完成。

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
- 本地构建通常没有固定路径下的 manifest，因此受管发现返回 no-op；`data/meta` 目录缺失时索引明确失败，目录存在但没有选中的 CSV 时返回 `skipped`。

准备结果产生 `storage.managed_meta.prepared` 结构化事件；索引命令的 stdout JSON 契约不变。该步骤只管理打包清单声明的文件，不替代下面的逐 CSV 身份和数据库投影预检。

### 规范目录与 Provider 路由

`litradar index` 对每个选中目录执行同一编排：

```text
read canonical CSV -> validate catalog contract
        |
        +-- catalog stem -> runtime index_provider_routes -> registered IndexContentProvider
        |
        +-- data/index/<stem>.sqlite         (content v4)
        +-- data/index-control/<stem>.sqlite (disposable control v1)
                    |
                    v
acquire provider-scoped lease -> fetch canonical batches
-> commit canonical content -> commit opaque checkpoint
-> publish provider-neutral change manifest -> release lease
```

目录验证在 Provider 请求前拒绝未知列、重复/非法 `catalog_id`、非法 ISSN、重复别名和不规范文本。索引路由来自 `auth.sqlite.runtime_settings.index_provider_routes`；摘要页和全文顺序分别来自带 default 与 catalog overrides 的运行设置。内容库和目录都不知道实际 Provider。

“分进程注册”只表示同一个 `litradar` 二进制在不同命令边界构造不同的内存注册表：`index` 进程注册索引实现，`serve` 的 API 进程注册摘要页/全文实现。它不是多服务部署，也不表示 Provider 自动回退。管理 API 按相同逻辑名称聚合这些注册，形成供前端过滤选项的 capability 目录。

Provider 只能返回规范 `JournalDraft`、`IssueDraft`、`ArticleDraft` 和 opaque checkpoint。`litradar-index` 负责校验、稳定 ID、合并、SQLite 事务和 outbox。内容先提交、checkpoint 后提交；控制提交失败时重跑会依靠规范 alias 幂等收敛。

### 索引数据库

每个 CSV 对应 `data/index/<csv_stem>.sqlite`。v4 内容库只包含规范期刊、期次、文章、identity aliases、查询/FTS 投影和事务性文章变更 outbox。它不包含 Provider、URL、checkpoint、lease 或运行统计。

`data/index-control/<csv_stem>.sqlite` 是可丢弃的 Provider-scoped checkpoint/lease 库。删除后会重新抓取，但不会改变内容 ID 或复制已有文章。内容库需要备份，控制库明确不备份。详见[数据库参考](reference/database.md)。

### 认证与业务数据库

`data/auth.sqlite` 保存：

- 用户、会话和访问令牌
- CNKI 用户会话
- 收藏夹和收藏文章
- 通知/追踪设置
- 全局运行配置，包括 Provider 路由、服务器安全和日志设置
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

1. `index_provider_routes` 为该目录选择 `scholarly` 索引能力。
2. Provider 接收规范 `JournalCatalogEntry`，Crossref 按 ISSN 提供文章主列表。
3. Crossref 对所有 ISSN 返回 404 时，OpenAlex 解析 source 并作为文章列表 fallback。
4. OpenAlex 按 DOI 增强元数据；Semantic Scholar 增强 OA 和缺失摘要；所有上游 URL 在映射边界丢弃。
5. Provider 返回规范 batch，`litradar-index` 写入关系表、`article_listing` 和 `article_search`。
6. `--update` 生成变更清单。

具体请求和字段优先级见 [Scholarly 数据源](reference/sources/scholarly.md)。

### CNKI 索引和全文

CNKI 元数据 Provider 使用 overseas 页面和接口生成规范内容；页面 filename 和详情 URL 只存在于一次适配调用中。按用户全文获取是独立的 `zjlib_cnki` 在线能力，使用当前用户已有的浙江图书馆会话，与索引 Provider 无关。详见 [CNKI 数据源](reference/sources/cnki.md)。

### 文章在线访问

内容库只向 API 提供规范 `ArticleLocator`。在线摘要页和全文分别按当前 CSV stem 的运行时顺序选择声明了相应能力的 Provider：

```text
browser -> stable LitRadar action URL -> load ArticleLocator
        -> use catalog override or inherit default order
        -> try eligible providers with timeout/fallback
        -> validate HTTPS host or bounded PDF
        -> 307/PDF + Cache-Control: private, no-store
```

默认 `scholarly → cnki` 是摘要能力的有序 fallback：先尝试 scholarly，遇到超时、未找到、临时失败或无效结果才继续 CNKI；它不是索引来源映射。catalog override 完整替换默认顺序，显式空数组禁用该 CSV 的动作。上游目的地不会出现在 `/access`、文章响应或索引库中。动作调用也不更新文章、outbox、checkpoint、认证会话或文件缓存。Provider 注册携带精确的运行时跳转域名 allowlist，API 不按 Provider 名称硬编码域名。

前端的“文章详情”是展示已经存入 LitRadar 的题名、作者、期刊、摘要等本地元数据的弹窗，不是第三种在线 Provider capability，也没有 `/detail` 动作路由。“查看摘要页”才会触发上述在线解析和外部跳转。

### 检索

1. 浏览器加载 Rust 提供的静态前端，并通过 `app/lib/api/` 同源调用 `/api/*`。
2. API 根据可选 `db` 选择一个 `data/index/*.sqlite`。
3. 列表与过滤使用 `article_listing`，详情从规范关系表读取。
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
- `runtime_settings`：外部元数据 key 池、按 CSV 的索引/在线 Provider 路由、CORS、MCP、Cookie 和日志策略
- `notification_settings`：每个用户的 AI、PushPlus 和追踪偏好
- 固定前端网络边界：浏览器同源，`next dev` 固定代理到 `127.0.0.1:8001`，生产静态导出
- 部署密钥文件：只用于认证和解密数据库中的秘密值

生产应用不读取 LitRadar 自定义环境变量。固定镜像路径和隐藏父子进程关联参数都是不可配置的内部协议；CLI 路径/监听/并发参数、部署密钥文件和测试工具输入仍保留各自边界。

来源、默认值和优先级见[运行配置](reference/configuration.md)。

## 数据库迁移

所有正式子命令在业务访问前执行所需的版本化 SQLite 迁移：

1. 解析路径和参数。
2. 检查 `PRAGMA user_version`。
3. 认证库在独立 `BEGIN IMMEDIATE` 事务中逐版本迁移。
4. 内容索引只接受新建/空 v0 或精确 v4；非空 v0 及 v1–v3 明确要求人工备份、移动或删除点名文件后重建。
5. 控制库按 v1 创建，可随时删除并重建。
6. 遇到未来版本或失败立即退出，不自动删除或改写文件。

`litradar serve` 先完成一次存储迁移、密钥验证和 HTTP 准备，再启动监听器与立即执行的首个调度 tick。普通查询仓库不负责 DDL。

## 同步工作与异步服务

SQLite、PBKDF2、阻塞 HTTP、文件系统和手动推送编排是同步工作。HTTP 组件通过 `ApiState` 的有界阻塞执行器把这些工作送入 Tokio blocking pool，避免在路由或 MCP future 中直接阻塞运行时。内嵌调度 tick 也通过 `spawn_blocking` 运行；按需子命令使用同步作业模型。

HTTP 组件共享的 `blocking executor` 有 8 个 permit。除此之外，手动周推在每个 `litradar serve` 进程内按 `auth.sqlite` 路径设置 1 个 admission slot：同一用户重复启动会复用当前 running job，不同用户竞争同一 storage instance 时立即收到 `503`，因此最多 1 个 manual API job 等待或占用共享 permit。该边界不排队、不持久化，也不是 `cross-process` 锁；独立调用的 `litradar notify`、`litradar push` 或计划任务子进程不受它协调。

## 部署边界

默认 Compose 只运行一个非 root、只读根文件系统且丢弃全部 Linux capabilities 的 `litradar` 容器，并把唯一 HTTP 入口 `127.0.0.1:8000` 发布到宿主机 loopback。公网部署必须增加 TLS 反向代理和共享限流，不能把默认端口直接改为所有网卡。详见 [Docker 部署](operations/docker.md)和[安全说明](operations/security.md)。
