# 系统架构

本文档说明 LitRadar 当前的组件边界、进程模型、数据流和持久化结构。命令参数见 [CLI 参考](../reference/cli.md)，具体部署步骤见 [Docker 部署](../operations/docker.md)。

## 总览

```text
browser
   |
   v
Caddy gateway -> client-web (Wails Server Build + Vite SPA)
   |
   +-- server: litradar serve (apps/server)
   |-- HTTP component: REST (huma) / OpenAPI docs / MCP
   |-- embedded scheduler: persistent cursor, claims and heartbeats
   |       |
   |       +-- current executable -> litradar index  (transient child)
   |       +-- current executable -> litradar notify (transient child)
   |       +-- current executable -> litradar push   (transient child)
   |
   +-- data/auth.sqlite
   +-- data/telemetry.sqlite                     (disposable load telemetry)
   +-- data/index/*.sqlite
   +-- data/index-control/index-batches.sqlite  (disposable project batch/catalog ledger)
   +-- data/index-control/<catalog>.sqlite       (disposable provider anchors/run checkpoints/leases)
   +-- data/index-work/scholarly/                (disposable Crossref workset)
   +-- data/push_state/*.changes.json

/usr/share/litradar/meta  (immutable image bundle)
              |
              +-- startup preparation -> data/meta/*.csv  (persistent copies)
                                           |
                                           +-- litradar index -> data/index/*.sqlite
                                                               -> *.changes.json -> litradar notify / litradar push
```

服务端 `apps/server` 是根 Go 模块中的 API/任务入口，不包含 UI。`apps/client` 与服务端共用根 Go 模块，使用 Wails v3，唯一前端为 `apps/client/frontend`；Vite 生成的 `dist` 仅嵌入客户端。Node.js 只参与开发与构建，服务端和 Web 客户端运行镜像均不需要 Node.js。

浏览器通过 Caddy 同源访问页面与 API。Windows x64、Android arm64-v8a 原生客户端连接用户配置的服务端 URL，使用账号密码或 OIDC 登录后换取 90 天有效的 `device:` 设备令牌，见[客户端与登录](../guides/clients.md)。发布不覆盖 iOS、macOS 或 Linux 桌面端；Wails v3 的 Android 支持为官方实验性功能。

## 运行进程

| 入口                 | 生命周期   | 职责                                                        |
| -------------------- | ---------- | ----------------------------------------------------------- |
| `litradar serve`     | 唯一常驻   | HTTP 服务与内嵌调度；统一负责准备、信号、关闭和组件失败传播 |
| `litradar index`     | 按需或调度 | 读取期刊 CSV、请求上游、写索引库和变更清单                  |
| `litradar cfp`       | 按需       | 导入或刷新期刊征稿原文                                      |
| `litradar notify`    | 按需或调度 | AI 选择后发送 PushPlus                                      |
| `litradar push`      | 按需或调度 | AI 选择后写入用户追踪文件夹                                 |
| `litradar scheduler` | 按需       | 校验任务或立即执行一个已保存的类型化任务                    |
| `litradar admin`     | 按需、本机 | 初始化管理员、维护密文、备份、离线恢复和索引存储优化        |
| `litradar openapi`   | 按需       | 把由 Go 类型生成的 OpenAPI 3.1 输出到 stdout 或文件         |

Docker Compose 运行 `server`、`client-web` 和 `gateway`。只有 `server` 挂载数据与密钥；`client-web` 只连接内部前端网络。Caddy 把 API、MCP、文档、OpenAPI 和健康检查路径转发到 `server`，其他页面交给客户端，并对未认证的 Wails 运行时端点 `/wails` 返回 404；完整路由见 [Docker 部署](../operations/docker.md)。服务端子任务通过同一 `litradar` 可执行文件运行，外部辅助程序为 Obscura 和 `pdftotext`。

SIGINT 或 SIGTERM 会同时通知 HTTP 与调度组件。若计划任务子进程正在运行，调度器先终止并等待该子进程，将运行状态持久化为 `cancelled`，且不再启动剩余步骤。HTTP、心跳或调度组件意外退出时，组合根会关闭另一组件并以非零状态退出，避免半可用进程继续运行。

## 日志流

服务端二进制在分发子命令前从 `data/auth.sqlite.runtime_settings` 只读加载 `log_format` 和 `log_filter`，再安装一个进程级 Go slog 日志处理器。数据库或表尚不存在时使用安全默认值；已存在但非法的值会让进程在业务工作前失败。所有包只产生结构化事件，不各自初始化日志处理器或写应用日志文件：

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

HTTP 外层先移除不受信的 `X-Request-Id`，再生成并返回服务器 UUID；请求内交给执行器或后台 goroutine 的工作重新进入同一 span。成功健康检查不产生逐请求终态事件。调度 span 使用 `run_id` 关联任务；父进程把该值作为经过校验的隐藏内部参数交给规范子进程，应用在公共命令解析前移除参数并记录 `parent_run_id`。它不出现在 help 中，也不是运维配置。子进程 `stderr` 直接继承到同一容器 sink，`stdout` 仍保留给结构化业务结果。

队列饱和时业务线程不等待；正常关闭最多等待 1 秒排空队列，并直接报告精确丢失行数。浏览器静态客户端没有日志采集端点，只在本地控制台记录不含错误消息和内容的白名单对象。完整字段、配置、保留、隐私和事故流程以[日志运维](../operations/logging.md)为准。

## 源码归属

`apps/server/` 拥有服务端代码、`assets/`、Dockerfile、构建检查脚本和专用测试；`apps/client/` 拥有客户端代码、构建脚本和专用测试。根 `scripts/`、`tests/` 只保留跨应用工具、发布协调与共享 `tests/data/`。根 Go/pnpm 工作区、部署入口和运行时 `data/` 保持共享；源码资源迁移不改变发行包的 `assets/meta` 布局。

## Go 包

| 包                                                                                                  | 职责                                                                                                                                                                  |
| --------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `apps/server`                                                                                       | 应用可执行文件与公开命令分发                                                                                                                                          |
| `apps/server/internal/runtime`                                                                      | 服务组装、信号、调度和进程监督；`runtime/observability`、`runtime/logfilter` 提供进程级日志处理与过滤                                                                 |
| `apps/client`                                                                                       | 共用根 Go 模块的 Wails 客户端；原生端与 Web host 共享 Vite UI                                                                                                         |
| `apps/server/internal/api`、`apps/server/internal/mcp`、`apps/server/internal/openapi`              | HTTP/MCP 契约、认证边界和 schema；REST 操作经 huma（`humago`）注册到标准库 `ServeMux`，OpenAPI 由 Go 类型生成                                                         |
| `apps/server/internal/auth`、`apps/server/internal/cli`                                             | 身份服务与命令编排                                                                                                                                                    |
| `apps/server/internal/domain`、`apps/server/internal/provider`                                      | 领域值、Provider 接口和能力注册表                                                                                                                                     |
| `apps/server/internal/index`、`apps/server/internal/sources`、`apps/server/internal/transport`      | 索引编排与多进程 worker、上游适配器（`sources/cnki`、`jfbym`、`scholarly`、`zjlib`）和出站 HTTP                                                                       |
| `apps/server/internal/recommend`、`apps/server/internal/delivery`、`apps/server/internal/scheduler` | 推荐、持久投递和定时任务                                                                                                                                              |
| `apps/server/internal/cfp`、`apps/server/internal/citation`                                         | 征稿采集与引用导出                                                                                                                                                    |
| `apps/server/internal/telemetry`                                                                    | 负载遥测：采集路由/MCP 负载与检索形态，写入本地 `data/telemetry.sqlite`                                                                                               |
| `apps/server/internal/storage`、`apps/server/internal/platform`                                     | 持久化与平台设施；`storage/index` 负责稳定 ID、合并、内容/控制库事务和 outbox；普通 CRUD 查询由 sqlc 生成到 `apps/server/internal/storage/sqlc/`，FTS 与动态 SQL 手写 |
| `apps/server/internal/compat`、`apps/server/internal/testkit`                                       | `compat/jsonvalue` 把标准 JSON 编码适配为持久化字符串值；`testkit` 只供测试和 `litradar-fixture` 使用                                                                 |

应用只有一个公开可执行文件。测试夹具命令单独位于 `apps/server/cmd/litradar-fixture`，sqlc schema 快照生成器位于 `apps/server/cmd/sqlc-schema`，两者都不随发行包分发。原生辅助程序是显式的运行时依赖。

## 持久化边界

### 元数据输入

`data/meta/*.csv` 是 LitRadar 维护的 Provider 无关期刊目录。每行使用不可变 `catalog_id`，并包含规范标题、印刷/电子/全部 ISSN、标题别名、领域和排名字段。目录中没有 `source`、Provider 名称、上游 ID、URL、路由或检查点。

目录格式、规范化和变更规则见[索引与 Provider 契约](../reference/index-provider-contract.md)。CSV 文件名 stem 同时决定稳定内容库名；更换 Provider 不改文件名或 `catalog_id`。

### 受管目录生命周期

发布镜像（`/usr/share/litradar/meta`）或二进制发行包（可执行文件旁的 `assets/meta`）携带不可变的官方 CSV 与 `bundle-manifest.json`。`litradar serve` 和普通 `litradar index` 在认证库迁移之后、验证密钥和加载运行设置之前，把 bundle 准备到持久的 `<project-root>/data/meta`：缺失文件写入，已知旧版升级，用户自定义的同名文件保留并报告冲突，清单外文件不读也不改。已应用版本记录在 `data/auth.sqlite.managed_meta_catalogs`，认证库记录的版本高于当前 bundle 时拒绝降级启动。多进程索引 worker 不重复准备；本地构建没有 manifest 时该步骤为 no-op。

查找顺序、逐文件分类、降级和退役文件处理见[运行配置](../reference/configuration.md#官方-meta-打包路径)。该步骤只管理打包清单声明的文件，不替代下面的逐 CSV 身份和数据库投影预检。

### 规范目录与 Provider 路由

`litradar index` 先把本次选择的 CSV 各读取一次，同时解析规范条目并对同一份精确字节计算摘要；随后在项目级 batch ledger 中取得全局 lease。默认 `--resume` 只接管 correctness inputs 完全兼容的 active batch。成功结束的 batch 不会被下一次命令复用，因此下一次独立更新仍会检查全部选中 journal。

```text
freeze ordered CSV selection -> validate catalog contracts
        |
        +-- data/index-control/index-batches.sqlite (batch schema v2 + global lease)
        |       pending -> indexing -> manifest_prepared
        |                            -> manifest_published -> notifying -> completed
        |
        +-- catalog stem -> runtime index_provider_routes -> registered provider.IndexContent
        |
        +-- data/index/<stem>.sqlite         (content v10)
        +-- data/index-control/<stem>.sqlite (disposable control v5)
                    |
                    v
acquire provider-scoped lease
-> skip only same-batch completed journal, otherwise read committed anchor or matching batch run
-> Provider fetch(mode, committed anchor, traversal checkpoint)
-> commit canonical content
-> Continue: advance traversal checkpoint
   Complete: atomically remove run and replace committed anchor with completed_batch_id
-> --update only: persist exact manifest intent, publish bytes, acknowledge cursor, optionally notify
-> release lease
```

batch compatibility 包含 CSV 的选择方式、顺序和精确内容、Provider route、同步模式、遗留 issue-batch 恢复值以及 notify 选项；worker/process 数、timeout、代理和凭据不参与。issue-batch 继续存在只是为了匹配旧 active batch 和既有 fingerprint，当前 Provider 不读取它来决定请求、分批、并发或内存。任一 correctness input 改变都会在 Provider 访问前 fail closed。`--no-resume` 是显式放弃 active batch：只清理该 batch 自有的运行 checkpoint，再从既有 committed anchor 创建新 batch；不会删除内容、anchor、outbox 或已经发布的 manifest。

崩溃恢复按 durable phase 前进：`indexing` 先跳过同 batch 已提交 anchor，再接管匹配 checkpoint；`manifest_prepared` 重发持久化的精确字节；`manifest_published` 只进入 notify 或完成；`notifying` 由持久化的 attempt ID 和 typed handoff 状态驱动，Unknown 结果需要 operator 显式确认。若 journal anchor 已提交但 catalog phase 尚未推进，same-batch marker 会阻止 Provider 重放；若文件 rename 或 outbox acknowledgement 已完成但 phase 尚未推进，重放仍使用同一 manifest intent。notify handoff 的状态判定、attempt 复用和 `--acknowledge-unknown-notify` 规则见 [CLI 参考](../reference/cli.md#实时恢复与增量同步)。

目录验证在 Provider 请求前拒绝未知列、重复/非法 `catalog_id`、非法 ISSN、重复别名和不规范文本。索引路由来自 `auth.sqlite.runtime_settings.index_provider_routes`；摘要页和全文顺序分别来自带 default 与 catalog overrides 的运行设置。内容库和目录都不知道实际 Provider。

“分进程注册”只表示同一个 `litradar` 二进制在不同命令边界构造不同的内存注册表：`index` 进程注册索引实现，`serve` 的 API 进程注册摘要页/全文实现。它不是多服务部署，也不表示 Provider 自动回退。管理 API 按相同逻辑名称聚合这些注册，形成供前端过滤选项的 capability 目录。

多进程索引使用私有 worker protocol v8，把同步模式、Provider-opaque 的 committed anchor 和 traversal checkpoint 随 journal assignment 写入可丢弃 request JSON；worker 不解析这些值，父进程独占 SQLite、batch ID 和提交顺序。worker 所需的秘密（captcha token、代理 URL、Scholarly key pool）不进入该文件、进程参数或 child 环境，只通过一次版本化 stdin bootstrap 交给对应 worker；同一管道随后传输 parent 的 durable commit ACK。秘密边界见[索引与 Provider 契约](../reference/index-provider-contract.md#provider-托管代理边界)。

在 Linux 上，独立进程组中的索引 worker 通过私有环境变量 `LITRADAR_WORKER_PARENT_PID` 获得启动者 PID。读取 worker 请求前，它校验归属与进程组隔离，随后由独立 goroutine 每 100 ms 检查一次父进程；父进程消失时，即使 Provider I/O 仍在阻塞，也会终止 worker 的整个进程组。正常完成时停止并等待该监视器。Windows 依靠父进程持有的 Job Object 监督。

Provider 只能返回规范 `JournalDraft`、`IssueDraft`、`ArticleDraft` 和 `ProviderProgress`。`Continue` 携带下一 traversal checkpoint；`Complete` 携带可空的 next anchor。两个字符串都保持 Provider-scoped、opaque，核心不解析 CNKI issue ID、Scholarly fingerprint 或上游 cursor。`apps/server/internal/index` 负责编排、契约校验和 worker 调度；`apps/server/internal/storage/index` 负责稳定 ID、合并、SQLite 事务和 outbox。内容先提交、控制状态后提交；控制提交失败时旧 anchor 保持不变，重跑依靠冻结窗口和规范 alias 幂等收敛。

### 索引数据库

每个 CSV 对应 `data/index/<csv_stem>.sqlite`。当前 v10 内容库只包含规范期刊、期次、文章、identity aliases、撤稿关系、查询/FTS 投影和事务性文章变更 outbox。v10 沿用 contentless FTS 布局：Go 先折叠重音和大小写，再按 `simple 0` 的字节规则切词并以空格连接，FTS5 内置的 `ascii` 分词器只按空格拆分；检索走全字段 FTS MATCH，查询操作数经过同一流程，原始元数据保持不变。内容库不包含 Provider、URL、anchor、checkpoint、lease 或运行统计。可接受的旧版本与 FTS 迁移见[数据库参考](../reference/database.md#内容库破坏性切换)，FTS 声明见 [`article_search`](../reference/database.md#article_search)。

分词不依赖 SQLite 扩展：连接上不注册自定义分词器，SQL 扩展加载保持关闭。

`data/index-control/index-batches.sqlite` 是项目级可丢弃 batch schema v2；`data/index-control/<csv_stem>.sqlite` 是 Provider-scoped v5 控制库。前者保存冻结输入指纹、catalog phase/outcome、精确 manifest intent、typed notify handoff/Unknown acknowledgement 和全局 lease，后者把成功 anchor 与运行中的 traversal checkpoint 分表保存并绑定 batch ID。删除全部控制状态后没有可信 batch、成功边界、handoff 或 traversal，下一次运行安全退回完整抓取，但不会改变内容 ID 或复制已有文章；operator 也同时承担失去待完成 handoff 证明的风险。切换 Provider 使用新的 namespace，同样从无 anchor 状态开始。内容库需要备份，两类控制库都明确不备份。详见[数据库参考](../reference/database.md)。

### 认证与业务数据库

`data/auth.sqlite` 保存：

- 用户、会话、访问令牌、邀请码和 OIDC 身份绑定（`user_identities`）
- 浙江图书馆（ZJLib）CNKI 用户会话（`cnki_sessions`）
- 收藏夹、收藏文章和关系图收藏（`relation_graph_favorites`）
- 通知/追踪设置
- 只追加的安全审计（`security_audit_events`）
- 全局运行配置，包括 Provider 路由、服务器安全和日志设置
- 类型化定时任务、运行槽和心跳
- 投递 checkpoint、run、item、dedupe 和 workflow lease
- 期刊征稿原文、来源、期刊身份和导入记录
- 系统公告

该库不保存 32 字节部署密钥；受保护的集成凭据以认证密文写入数据库，密钥通过文件单独提供。

### 外部变更清单与旧导入源

| 路径                                | 所有者                                                       |
| ----------------------------------- | ------------------------------------------------------------ |
| `data/push_state/<db>.changes.json` | `litradar index --update` 生成；每周更新、通知和追踪共同读取 |
| `data/push_state/<db>.json`         | 启动时只读导入并原样保留的旧 notify 状态                     |
| `data/folder_push_state/<db>.json`  | 启动时只读导入并原样保留的旧 push 状态                       |

变更清单是新文章分发的输入，不是可从文章日期实时重建的视图。读取方只以必填的 `db_name` 识别目标数据库，不使用保存的文件系统路径作为身份回退。可变投递状态只存在于 `auth.sqlite`；运行时不会写 `<db>.json` 或固定 `.tmp`。

### 负载遥测库

`data/telemetry.sqlite` 只保存按小时聚合的匿名负载：HTTP 路由模板、MCP 工具调用和检索形态（入口、模式、词数桶等），不保存用户身份、IP、原始路径或检索文本。`litradar serve` 在 `telemetry_enabled` 开启时于内存聚合并定期写入，管理员在管理面板的“负载”分区查看。该库可删除后重建，备份始终排除它。表结构见[数据库参考](../reference/database.md#负载遥测库)，设置见[运行配置](../reference/configuration.md#负载遥测设置)。

## 主要数据流

### Scholarly 索引

1. `index_provider_routes` 为该目录选择 `scholarly` 索引能力。
2. Provider 接收 contract v3 的规范 `JournalCatalogEntry` 与同步 context；Crossref 按 ISSN 以 created 日期分片收集完整清单（不排序、每次 225 条），计数校验通过后在本地工作集中按期次排序；整刊 404 或为空时尝试下一个 ISSN。
3. 全部 ISSN 都没有可用 Crossref 清单时，OpenAlex 解析 source 并作为文章列表 fallback。
4. 对当前输出页先按 DOI 批量请求 Semantic Scholar，再只为仍缺字段的记录请求 OpenAlex；所有上游 URL 在映射边界丢弃。
5. 增量运行覆盖从冻结的远端头部到成功 anchor 的规范期次，并完整包含该边界期次；update 日期条件只缩小候选，不能替代边界证明。
6. Provider 返回规范 batch 和 Continue/Complete 进度，`apps/server/internal/storage/index` 写入关系表、`article_listing` 和 `article_search`，整刊完成后才推进 anchor。
7. `--update` 生成变更清单；`--full-rescan` 覆盖完整历史但不生成该清单。

具体请求和字段优先级见 [Scholarly 数据源](../reference/sources/scholarly.md)。

### CNKI 索引和全文

默认国内 `cnki` 元数据 Provider 使用 NZKPT 的 `navi.cnki.net` / `kns.cnki.net` 页面和接口生成规范内容。页面 filename、详情 URL 和 captcha 状态只存在于一次适配调用中。按用户全文获取是独立的 `zjlib` 在线能力，使用当前用户已有的浙江图书馆会话，与索引 Provider 无关。详见 [CNKI 数据源](../reference/sources/cnki.md)。

### 征稿追踪

征稿追踪使用维护目录中的期刊身份，但不依赖文章索引是否已有内容。后端负责征稿采集、原文解析、状态计算和持久化；前端通过认证 API 浏览，不在浏览器中运行采集器，也不把种子数据打包进 JavaScript。征稿状态随查询时刻计算，日期缺失不表示长期开放。数据归属、刷新流程与限制见[征稿追踪架构](cfp-tracking.md)。

### 文章在线访问

内容库只向 API 提供规范 `ArticleLocator`。在线摘要页和全文分别按当前 CSV stem 的运行时顺序选择声明了相应能力的 Provider：

```text
browser -> stable LitRadar action URL -> load ArticleLocator
        -> use catalog override or inherit default order
        -> try eligible providers with timeout/fallback
        -> validate HTTPS host or bounded PDF
        -> 307/PDF + Cache-Control: private, no-store
```

每个顺序是请求时的有序 fallback，不是索引来源映射；catalog override 完整替换默认顺序，显式空数组禁用该 CSV 的动作。默认值与语法见[运行配置](../reference/configuration.md#provider-路由语法)。上游目的地不会出现在 `/access`、文章响应或索引库中。动作调用也不更新文章、outbox、checkpoint、认证会话或文件缓存。Provider 注册携带精确的运行时跳转域名 allowlist，API 不按 Provider 名称硬编码域名。

前端的“文章详情”是展示已经存入 LitRadar 的题名、作者、期刊、摘要等本地元数据的弹窗，不是第三种在线 Provider capability，也没有 `/detail` 动作路由。“查看摘要页”才会触发上述在线解析和外部跳转。

### 检索

1. 浏览器通过 Caddy 加载 Wails Web host 提供的 Vite 前端，并通过 `apps/client/frontend/lib/api/` 同源调用 `/api/*`。
2. API 根据可选 `db` 选择一个 `data/index/*.sqlite`。
3. 列表与过滤使用 `article_listing`，详情从规范关系表读取。
4. `q` 默认先转义为一个 FTS5 字面短语再使用 `article_search MATCH`；只有显式 `search_mode=advanced` 才解释 FTS5 运算符，非法表达式作为可纠正的 400 输入错误返回。
5. API 把文章、期刊和期次的 63 位正整数哈希 ID 序列化为十进制字符串，格式见 [API 参考](../reference/api.md#id-与分页)。

### 认证

浏览器登录后使用 `HttpOnly` 的 `litradar_session` Cookie；服务端配置 OIDC 时，浏览器也可通过 OIDC 单点登录。原生客户端用账号密码或 OIDC 换取 `device:` 设备令牌；MCP 等外部客户端使用用户在设置中创建的 Bearer 访问令牌。首个管理员只能通过本机 `litradar admin bootstrap` 创建；公开注册始终要求邀请码。OIDC 部署见 [authentik 单点登录](../operations/authentik.md)，原生登录见[客户端与登录](../guides/clients.md)。

### 通知和追踪

`notify`、`push` 和手动周报按 `(workflow, db_name)` 认领 SQLite run 与 workflow lease，从变更清单或 checkpoint 快照差异建立持久 run item，再用用户级偏好和主备 AI 选择文章。外部副作用前先以唯一约束预留文章去重；结果不明确时落为 `unknown` 且不会自动重放。`index --notify` 的 parent attempt 只改变外层 run 身份，不改变文章去重身份。副作用顺序与恢复规则见[通知与追踪](../guides/notifications.md#副作用顺序)。

## 配置层次

系统使用多种明确分离的配置来源：

- CLI 参数：`litradar` 子命令的路径、端口、并发和一次运行覆盖
- `runtime_settings`：外部元数据 key 池、Provider 代理、按 CSV 的索引/在线 Provider 路由、OIDC、CORS、MCP、Cookie、审计保留、AI Endpoint 目录、投递并发、负载遥测和日志策略
- `notification_settings`：每个用户的 AI、PushPlus 和追踪偏好
- 前端网络边界：浏览器同源；开发服务器（`vp dev`）默认代理到 `127.0.0.1:8001`，可用 `LITRADAR_BACKEND_URL` 覆盖；发布构建输出由 Web host 嵌入的静态 `dist/`
- 部署密钥文件：只用于认证和解密数据库中的秘密值

部署后的应用不把 LitRadar 自定义环境变量作为通用配置中心。固定镜像路径和隐藏父子进程关联参数都是不可配置的内部协议；CLI 路径/监听/并发参数、部署密钥文件和测试工具输入仍保留各自边界。数据库 token 为空时，`LITRADAR_CNKI_CAPTCHA_TOKEN` 是国内 CNKI 单次索引探测的唯一来源凭据例外，并在父进程解析后从 child 环境移除。

来源、默认值和优先级见[运行配置](../reference/configuration.md)。

## 数据库迁移

正式子命令在业务访问前完成各自需要的存储准备：先解析路径和参数，检查 `PRAGMA user_version`，再执行受支持的迁移或结构预检。认证库按版本在独立的 `BEGIN IMMEDIATE` 事务中迁移；普通查询仓库不负责数据定义操作。

认证库、内容库、项目批次账本、目录控制库和负载遥测库分别维护版本；当前版本与迁移边界见[数据库参考](../reference/database.md#连接和版本)，需要重建的旧内容库处理步骤见[内容库破坏性切换](../reference/database.md#内容库破坏性切换)。

项目批次账本与目录控制库可以重建，但删除它们会丢失恢复进度和已发布通知的交接记录。遇到未来版本或迁移失败时，命令会退出，不会自动删除文件。`serve` 完成存储迁移、密钥验证和 HTTP 准备后，才启动监听器及首轮调度。

## 有界执行器与后台服务

SQLite、密码派生、HTTP 和文件系统操作通过有界 Go 执行器运行。API 分别限制存储、上游请求和密码派生任务，容量分别为 8、4、2，排队超时为 30 秒；取消和关闭通过 context 与显式生命周期管理。定义见[运行时准备](../../apps/server/internal/runtime/prepare.go)与 `apps/server/internal/platform/executor`。内嵌调度与手动投递由运行时监督器管理。

手动周报通过 SQLite 持久任务排队，API 返回 `202` 后由独立的[投递监督器](../../apps/server/internal/runtime/manual_delivery.go)启动子进程。同一用户重复启动会复用当前排队或活动任务；不同用户可以排队并在实例容量内并行。并发池由 `delivery_worker_concurrency` 控制，租约、版本比较和去重仍由存储层保证。状态、取消和不确定结果的确认规则见[API 参考](../reference/api.md#收藏与追踪)，配置见[运行配置](../reference/configuration.md#用户通知配置)。

### 跨调度轮次的任务执行

Cron 表达式语法见 [API 参考](../reference/api.md#管理接口)。内嵌调度器最多保留 4 个活动执行。长时间索引不会阻止发现后续到期任务：每轮先把到期时段持久化，再按空闲执行槽认领任务。多余工作保持待处理状态，不会提前取得认领租约；每个任务仍最多有一次活动运行。

手动与自动运行对每个任务共享同一个持久活动认领边界。手动历史与 cron 时段身份分开保存，不推进合并水位。

合并在待处理、已认领、运行中和终态记录之间保留最新的已知调度时段。回收过期且未启动的认领后会再次删除更早的待处理时段，因此延迟的 tick 和重启不会复活已被合并掉的工作。关闭合并的任务保留每个错过的时段。

服务负责活动执行集合，并在关闭或基础设施故障时协同取消、回收所有子进程。每次运行的心跳和进程树监督保持有效。一次性 `scheduler` 命令会等待自己已接纳的运行完成；每轮摘要汇总上一轮以来收集的结果，单次运行的终态事件则立即记录。

## 部署边界

默认 Compose 运行 `server`、`client-web`、`gateway` 三个容器，均使用只读根文件系统和 `no-new-privileges`；`server` 与 `client-web` 丢弃全部 Linux capabilities，网关只保留 `NET_BIND_SERVICE`。只有 Caddy 网关把 HTTP 入口发布到宿主机，默认 `127.0.0.1:8000`（`LITRADAR_BIND_ADDRESS`、`LITRADAR_PORT` 可改）；`server` 与 `client-web` 不发布端口。对外访问时必须启用 TLS 和共享限流，不能把默认 loopback 入口直接改为所有网卡。详见 [Docker 部署](../operations/docker.md)和[安全说明](../operations/security.md)。
