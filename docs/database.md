# 数据库结构说明

Paper Scanner 当前实际使用两类数据库：

1. **索引数据库**
   - 路径：`data/index/*.sqlite`
   - 来源：每个 `data/meta/*.csv` 对应生成一个 `.sqlite`
   - 作用：期刊、期次、文章、全文检索

2. **认证与业务数据库**
   - 路径：`data/auth.sqlite`
   - 作用：用户、访问令牌、收藏夹、通知设置、定时任务、公告

## 一、索引数据库

### 初始化参数

索引数据库在 `scripts/index/db/schema.py` 中初始化，当前会设置以下 pragma：

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
```

### 1. `journals`

期刊主表。

主要字段：

- `journal_id`：主键
- `library_id`：上游库 ID
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
- `csv_library`

主要用途：

- 为筛选页提供 `area`
- 为调试与回溯保留原始 CSV 信息

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

文章主表，是公开检索与全文跳转的核心数据源。

主要字段：

- 基本信息：
  - `article_id`
  - `journal_id`
  - `issue_id`
  - `sync_id`
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
  - `ill_url`
  - `link_resolver_openurl_link`
  - `email_article_request_link`
  - `permalink`
  - `full_text_file`
  - `libkey_full_text_file`
  - `nomad_fallback_url`
- 状态位：
  - `suppressed`
  - `in_press`
  - `open_access`
  - `within_library_holdings`
  - `unpaywall_data_suppressed`
  - `avoid_unpaywall_publisher_links`
- 其他来源字段：
  - `platform_id`
  - `retraction_doi`
  - `retraction_date`
  - `retraction_related_urls`
  - `expression_of_concern_doi`
  - `noodletools_export_link`
  - `browzine_web_in_context_link`
  - `content_location`
  - `libkey_content_location`

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
  - `idx_articles_in_press`
  - `idx_articles_suppressed`
  - `idx_articles_within_holdings`
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
- `within_library_holdings`
- `doi`
- `pmid`
- `area`

作用：

- 减少 `/api/articles` 多表联查成本
- 在 `listing_state` 就绪时优先服务列表检索

主要索引：

- `idx_article_listing_date_id`
- `idx_article_listing_area`
- `idx_article_listing_publication_year`
- `idx_article_listing_journal`
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

- 如果成功加载 `simple` 扩展，则会以 `tokenize = 'simple'` 创建
- 否则使用默认 FTS5 tokenizer

### 7. `listing_state`

单行状态表，用于标记 `article_listing` 是否可供查询使用。

字段：

- `id`，固定为 `1`
- `status`
- `updated_at`

当前代码会查询 `status = 'ready'` 来判断是否启用 `article_listing`。

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

## 二、认证与业务数据库 `data/auth.sqlite`

### 初始化参数

`scripts/api/auth_db.py` 初始化时会设置：

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
  ├── folders
  │    └── favorites
  ├── invite_codes (created_by / used_by)
  └── notification_settings

scheduled_tasks
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
- 收藏导出接口支持直接用原始令牌作为 `access_token` 查询参数

### 3. `folders`

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

### 4. `favorites`

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

### 5. `invite_codes`

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

### 6. `notification_settings`

通知与追踪配置表。

主要字段：

- `id`
- `user_id`
- `keywords`
- `directions`
- `delivery_method`
- `pushplus_token`
- `pushplus_template`
- `pushplus_topic`
- `pushplus_channel`
- `ai_base_url`
- `ai_api_key`
- `ai_model`
- `ai_system_prompt`
- `enabled`
- `created_at`
- `updated_at`

说明：

- 当前 API 只接受 `delivery_method = "folder"` 或 `"pushplus"`
- 这张表是 `notify`、`push` 与 `/api/tracking/push-weekly` 的真实订阅源

### 7. `scheduled_tasks`

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

- 由 APScheduler 在 API 进程内调度
- 执行方式是 shell 命令

### 8. `announcements`

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

PushPlus 通知运行状态。

### 3. `data/folder_push_state/<db>.json`

追踪文件夹推送运行状态。
