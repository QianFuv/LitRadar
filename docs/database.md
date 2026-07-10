# 数据库结构说明

Paper Scanner 当前实际使用两类数据库：

1. **索引数据库**
   - 路径：`data/index/*.sqlite`
   - 来源：每个 `data/meta/*.csv` 对应生成一个 `.sqlite`
   - 作用：期刊、期次、文章、全文检索、索引运行统计

2. **认证与业务数据库**
   - 路径：`data/auth.sqlite`
   - 作用：用户、访问令牌、CNKI 会话、收藏夹、通知设置、运行时配置、定时任务、公告

后端部署与正常运行入口已经切换到 Rust 服务。Rust API、worker 和 CLI 在业务访问前执行版本化迁移，并继续读取同一套 `data/`；数据库之外的状态文件格式不变。

## 数据库版本与迁移生命周期

认证库和每个索引库分别使用 SQLite `PRAGMA user_version` 记录 schema 版本。当前认证库版本和索引库版本均为 `1`。

- API 在绑定监听端口前迁移 `data/auth.sqlite` 和 `data/index/*.sqlite`
- `worker` 在进入调度循环前迁移；`scheduler`、`index`、`notify` 和 `push` 在参数验证完成、首次业务访问前迁移
- `index` 在执行结束后再次扫描本次新建的索引库；已经是当前版本的库不会执行写操作
- 普通 repository 连接只设置连接级 pragma 和执行数据查询，不创建表、不执行 `ALTER TABLE`，也不通过 schema introspection 决定迁移

每个待执行版本都在独立的 `BEGIN IMMEDIATE` 事务中完成，`user_version` 只在同一事务末尾更新。任何 DDL 或索引创建失败都会回滚该版本，启动命令以非零错误退出，不会继续提供部分升级后的服务。

如果数据库的 `user_version` 高于当前二进制支持的版本，启动会在切换 WAL 或执行其他写操作前拒绝该文件。此时应升级应用二进制，不能手工降低 `user_version`。生产升级前仍应备份 `data/auth.sqlite`、所有 `data/index/*.sqlite` 及关联状态目录；迁移失败时修复不兼容的旧 schema 或从备份恢复后重试。

## 一、索引数据库

### 初始化参数

索引数据库的版本状态和旧库升级由版本化 storage migration 管理；Rust 索引写入路径负责创建新库的当前 schema，`index` 命令会在结束前将新库纳入同一版本管理。连接会设置以下 pragma：

| Pragma | 值 | 说明 |
| --- | --- | --- |
| `journal_mode` | `WAL` | 允许并发读写 |
| `foreign_keys` | `ON` | 开启外键 |
| `synchronous` | `NORMAL` | 平衡安全与性能 |
| `busy_timeout` | `30000 ms` | 锁等待时间 |

时间字段约定：

- 索引数据库中的时间大多使用 `TEXT`
- 日期通常保存为 ISO-8601 字符串或上游原始日期文本

### 表关系概览

```text
journals (1) ---- (1) journal_meta
   |
   +---- (N) issues (1) ---- (N) articles
                               |
                               +---- article_listing   物化筛选辅助表
                               +---- article_search    FTS5 全文检索表

index_runs (1) ---- (N) index_path_stats
          |
          +---- (N) index_api_call_stats
```

### 1. `journals`

期刊主表。

主要字段：

- `journal_id`：主键
- `library_id`：数据源标识，当前通常为 `scholarly` 或 `cnki`
- `platform_journal_id`：上游平台期刊 ID；英文期刊通常为 ISSN，CNKI 为 `pykm`
- `title`
- `issn`
- `eissn`
- `scimago_rank`
- `cover_url`
- `available`
- `toc_data_approved_and_live`
- `has_articles`

主要索引：

- `idx_journals_issn`
- `idx_journals_library_id`
- `idx_journals_available`
- `idx_journals_has_articles`
- `idx_journals_scimago_rank`

### 2. `journal_meta`

保存 CSV 源文件元数据。

主要字段：

- `journal_id`：主键，同时外键到 `journals`
- `source_csv`
- `area`
- `csv_title`
- `csv_issn`
- `csv_library`：当前保存 CSV 中的 `source` 值

主要用途：

- 为筛选页提供 `area`
- 为调试与回溯保留原始 CSV 信息

主要索引：

- `idx_journal_meta_area`
- `idx_journal_meta_area_journal`

### 3. `issues`

期次表。

主要字段：

- `issue_id`：主键
- `journal_id`
- `publication_year`
- `title`
- `volume`
- `number`
- `date`
- `is_valid_issue`
- `suppressed`
- `embargoed`
- `within_subscription`

主要索引：

- `idx_issues_journal_year`
- `idx_issues_publication_year`

### 4. `articles`

文章主表，是检索与全文跳转的核心数据源。

主要字段：

- 基本信息：
  - `article_id`
  - `journal_id`
  - `issue_id`
  - `title`
  - `date`
  - `authors`
  - `abstract`
- 页码与标识：
  - `start_page`
  - `end_page`
  - `doi`
  - `pmid`
- 外部链接：
  - `permalink`
  - `content_location`，英文路径保存 OpenAlex、Crossref 或 DOI landing page；CNKI 路径保存详情页
  - `full_text_file`，英文路径保存 Semantic Scholar / OpenAlex OA URL；CNKI 路径保持为空，避免直接跳转权限控制入口
- 状态位：
  - `suppressed`
  - `in_press`
  - `open_access`，CNKI 路径保持为空，不做 OA 检测
  - `within_library_holdings`，保留给筛选与通知逻辑，新抓取数据通常为空
- 其他来源字段：
  - `platform_id`
  - `retraction_doi`

主要索引：

- 按时间与关联关系：
  - `idx_articles_journal`
  - `idx_articles_issue`
  - `idx_articles_date`
  - `idx_articles_date_id`
  - `idx_articles_journal_date_id`
  - `idx_articles_issue_date_id`
- 按可过滤状态：
  - `idx_articles_open_access`
  - `idx_articles_open_access_date_id`
  - `idx_articles_in_press`
  - `idx_articles_in_press_date_id`
  - `idx_articles_suppressed`
  - `idx_articles_suppressed_date_id`
  - `idx_articles_within_holdings`
  - `idx_articles_within_holdings_date_id`
- 按常用精确查询：
  - `idx_articles_doi`
  - `idx_articles_pmid`

### 5. `article_listing`

这是为检索接口准备的物化辅助表，不保存完整文章内容，只保留高频筛选字段。

字段：

- `article_id`
- `journal_id`
- `issue_id`
- `publication_year`
- `date`
- `open_access`
- `in_press`
- `suppressed`
- `within_library_holdings`，历史字段，新抓取数据通常为空
- `doi`
- `pmid`
- `area`

作用：

- 减少 `/api/articles` 多表联查成本
- 在 `listing_state` 就绪时优先服务列表检索

主要索引：

- `idx_article_listing_date_id`
- `idx_article_listing_area`
- `idx_article_listing_area_date_id`
- `idx_article_listing_publication_year`
- `idx_article_listing_journal`
- `idx_article_listing_journal_date_id`
- `idx_article_listing_issue`

### 6. `article_search`

SQLite FTS5 虚表，用于全文检索。

索引字段：

- `article_id`（`UNINDEXED`）
- `title`
- `abstract`
- `doi`
- `authors`
- `journal_title`

说明：

- Rust 索引初始化会从项目内 `libs/simple-*` 下发现 `simple` 扩展；如果加载成功，则以 `tokenize = 'simple'` 创建
- 如果扩展不存在或加载失败，建库不会中断，会使用默认 FTS5 tokenizer
- 查询层对 `q` 使用 SQLite FTS5 `MATCH ?`；中文、英文分词由 `article_search` 建表时的 tokenizer 决定，不做拼音查询展开
- 对已经存在的 `article_search`，`CREATE VIRTUAL TABLE IF NOT EXISTS` 不会重建 FTS 表；从默认 tokenizer 迁移到 `simple` tokenizer 需要单独重建全文索引

### 7. `listing_state`

单行状态表，用于标记 `article_listing` 是否可供查询使用。

字段：

- `id`，固定为 `1`
- `status`
- `updated_at`

Rust 索引初始化只创建该表，不会在空库或未完成构建时写入 `ready`。live/fixture 索引成功完成后会写入：

- `id = 1`
- `status = 'ready'`
- `updated_at = <本次索引时间戳>`

当前查询代码会读取 `status = 'ready'`，并确认 `article_listing` 至少有一行后才启用 `article_listing` 分支；缺少该状态或状态不是 `ready` 时会回退到 `articles` 联查分支。

### 8. `journal_year_state`

增量索引恢复表，记录某个期刊某一年的抓取是否完成。

字段：

- `journal_id`
- `year`
- `status`
- `updated_at`

主键：

- `(journal_id, year)`

### 9. `journal_state`

增量索引恢复表，记录某个期刊整体是否完成。

字段：

- `journal_id`
- `status`
- `updated_at`

### 10. `index_runs`

索引运行汇总表，记录每次 CSV 索引任务的整体结果。

主要字段：

- `run_id`：主键，格式为 `<csv_stem>-<uuid>`
- `csv_file`
- `started_at`
- `finished_at`
- `status`
- `total_journals`
- `succeeded_journals`
- `failed_journals`
- `resumed_journals`
- `error_summary`

### 11. `index_path_stats`

索引路径统计表，记录单本期刊在某条抓取路径上的执行统计。

主要字段：

- `run_id`：外键到 `index_runs`
- `source`
- `path`
- `journal_id`
- `journal_title`
- `status`
- `started_at`
- `finished_at`
- `works_count`
- `issues_count`
- `article_summaries_count`
- `article_details_count`
- `articles_written_count`
- `articles_deleted_no_authors_count`
- `error_type`
- `error_message`

主要索引：

- `idx_index_path_stats_run`
- `idx_index_path_stats_status`

### 12. `index_api_call_stats`

索引 API 调用统计表，记录每次运行中外部服务调用的逻辑次数、重试、错误和耗时。

主要字段：

- `run_id`：外键到 `index_runs`
- `source`
- `service`
- `endpoint`
- `method`
- `url_path`
- `journal_id`
- `journal_title`
- `logical_calls`
- `attempts`
- `successes`
- `failures`
- `retry_count`
- `status_codes_json`
- `transport_errors`
- `rate_limit_failures`
- `total_latency_ms`
- `error_samples_json`

主要索引：

- `idx_index_api_call_stats_run`
- `idx_index_api_call_stats_service`

## 二、认证与业务数据库 `data/auth.sqlite`

### 初始化参数

认证与业务数据库由 Rust storage migration 初始化。当前连接会设置：

| Pragma | 值 |
| --- | --- |
| `journal_mode` | `WAL` |
| `foreign_keys` | `ON` |

时间字段约定：

- 该库的时间大多保存为 `REAL`
- 值通常来自 `time.time()` 的 Unix 时间戳（秒）

### 表关系概览

```text
users
  ├── access_tokens
  ├── cnki_sessions
  ├── folders
  │    └── favorites
  ├── invite_codes (created_by / used_by)
  └── notification_settings

scheduled_tasks
runtime_settings
announcements
```

### 1. `users`

用户主表。

主要字段：

- `id`
- `username`
- `password_hash`
- `salt`
- `is_admin`
- `created_at`
- `updated_at`

说明：

- 首个注册用户会被设为管理员

### 2. `access_tokens`

用户访问令牌。

主要字段：

- `id`
- `user_id`
- `name`
- `token_hash`
- `expires_at`
- `created_at`

作用：

- 登录成功时生成名为 `login` 的令牌
- 用户可在设置页创建更多长期令牌
- 浏览器登录令牌通过 `HttpOnly` `ps_session` Cookie 传输；用户创建的长期令牌通过 Bearer 请求头用于外部客户端

### 3. `cnki_sessions`

当前用户的浙江图书馆 CNKI 会话表。

主要字段：

- `user_id`：主键，同时外键到 `users`
- `session_json`：序列化后的客户端会话状态
- `qr_uuid`：最近一次扫码登录 UUID
- `status`：常见值为 `empty`、`waiting_scan`、`active`、`expired`
- `token_expires_at`
- `created_at`
- `updated_at`
- `last_used_at`

主要索引：

- `idx_cnki_sessions_status`

说明：

- 每个 Paper Scanner 用户最多保存一条独立 CNKI 会话
- API 状态响应只派生安全元数据，例如过期时间和 cookie 名称，不返回 token 或 cookie 值
- 该表只用于中文 CNKI 全文 provider；英文数据库与 CCF 数据库不使用浙江图书馆 CNKI 会话

### 4. `folders`

用户文件夹。

主要字段：

- `id`
- `user_id`
- `name`
- `is_tracking`
- `created_at`
- `updated_at`

说明：

- `is_tracking = 1` 的文件夹表示用户当前追踪文件夹
- 每个用户同名文件夹受唯一约束限制

### 5. `favorites`

收藏明细表。

主要字段：

- `id`
- `user_id`
- `folder_id`
- `article_id`
- `db_name`
- `note`
- `created_at`

说明：

- `db_name` 指向来源索引库文件名，不是外键
- 同一用户 / 文件夹 / 文章 / 数据库组合会被去重

### 6. `invite_codes`

邀请码表。

主要字段：

- `id`
- `code`
- `created_by`
- `used_by`
- `used_at`
- `created_at`

说明：

- 普通用户最多只能生成一个邀请码
- 管理员可额外创建“无创建者”的后台邀请码
- 已使用的邀请码不能被后台删除

### 7. `notification_settings`

通知与追踪配置表。

主要字段：

- `id`
- `user_id`
- `keywords`
- `directions`
- `selected_databases`
- `delivery_method`
- `pushplus_token`
- `pushplus_template`
- `pushplus_topic`
- `pushplus_channel`
- `sync_to_tracking_folder`
- `ai_base_url`
- `ai_api_key`
- `ai_model`
- `ai_system_prompt`
- `ai_backup_base_url`
- `ai_backup_api_key`
- `ai_backup_model`
- `ai_backup_system_prompt`
- `ai_retry_attempts`
- `enabled`
- `created_at`
- `updated_at`

说明：

- 当前 API 只接受 `delivery_method = "folder"` 或 `"pushplus"`
- 这张表是 `notify`、`push` 与 `/api/tracking/push-weekly` 的真实订阅源

### 8. `scheduled_tasks`

管理员后台可维护的定时任务。

字段：

- `id`
- `name`
- `command`
- `cron`
- `enabled`
- `last_run_at`
- `last_status`
- `created_at`
- `updated_at`

说明：

- Docker 默认由 `worker --project-root /app --interval-seconds 300` 持续加载并按 cron 自动执行启用任务
- 立即执行和 dry-run 由 `scheduler run-once TASK_ID` 与 `scheduler dry-run-once TASK_ID` 触发
- 执行模式仍按 shell 命令处理，不会把 `runtime_settings` 注入命令环境

### 9. `runtime_settings`

管理员后台维护的外部元数据运行配置表。

字段：

- `key`：主键，使用 API 字段名
- `value`
- `updated_at`

说明：

- 当前受管理的 `key` 包括 `openalex_api_key_pool`、`semantic_scholar_api_key_pool`、`crossref_mailto_pool`、`cors_allowed_origins`、`mcp_allowed_hosts`、`mcp_allowed_origins` 和 `secure_cookies`
- Rust API、索引命令和调度任务会读取该表并应用运行时配置
- 数据库中没有值时使用代码默认值；运行时配置不从进程环境变量回退

### 10. `announcements`

系统公告表。

字段：

- `id`
- `title`
- `message`
- `priority`
- `enabled`
- `created_at`
- `updated_at`

说明：

- 后台管理使用 `list_all_announcements()`
- 前台首页只读取启用公告，并按优先级 `high -> normal -> low` 加时间倒序排列

## 三、数据库之外的状态文件

虽然不属于 SQLite 表结构，但以下文件与数据库行为强相关：

### 1. `data/push_state/<db>.changes.json`

索引增量更新生成的变更清单，是：

- 每周更新页面
- PushPlus 通知
- 追踪推送

这三条链路的共同输入。

### 2. `data/push_state/<db>.json`

PushPlus 通知、每周更新和 API 手动推送状态。Rust `notify` 默认使用 `data/push_state/`。

Rust `push` 命令默认使用 `data/folder_push_state/<db>.json` 保存追踪文件夹推送状态；历史 Python 运行路径也可能留下同目录文件。
