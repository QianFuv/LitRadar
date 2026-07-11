# 数据库参考

LitRadar 使用多个 SQLite 文件和两个状态目录。本页说明当前逻辑 schema、迁移版本和关键字段语义；备份操作见[备份与恢复](../operations/backup.md)。

## 文件布局

| 路径                      |                数量 | 责任                             |
| ------------------------- | ------------------: | -------------------------------- |
| `data/index/*.sqlite`     | 每个元数据 CSV 一个 | 期刊、期次、文章、FTS5、索引统计 |
| `data/auth.sqlite`        |                一个 | 用户、收藏、配置、任务、公告     |
| `data/push_state/`        |           多个 JSON | 变更清单、notify 和手动推送状态  |
| `data/folder_push_state/` |           多个 JSON | push 追踪文件夹状态              |

## 连接和迁移

所有正式后端入口在业务访问前运行版本化 migration。当前版本：

| 数据库      | `PRAGMA user_version` |
| ----------- | --------------------: |
| 认证/业务库 |                     4 |
| 索引库      |                     1 |

连接设置：

| Pragma/设置    | 值       |
| -------------- | -------- |
| `foreign_keys` | `ON`     |
| `journal_mode` | `WAL`    |
| `synchronous`  | `NORMAL` |
| busy timeout   | 30 秒    |

迁移按版本使用独立 `BEGIN IMMEDIATE` 事务，并在同一事务末尾更新 `user_version`。数据库版本高于当前二进制时，在业务写入前拒绝；不要手工降低版本。

API 在迁移后才绑定端口，worker 在迁移后才进入循环。`index` 结束后再次把本次新建的库纳入版本检查。普通 repository 连接不执行 DDL。

## 索引数据库

### 关系

```text
journals (1) ---- (1) journal_meta
   |
   +---- (N) issues (1) ---- (N) articles
                               |
                               +---- article_listing
                               +---- article_search (FTS5)

index_runs (1) ---- (N) index_path_stats
          |
          +---- (N) index_api_call_stats

journal_state / journal_year_state / listing_state
```

日期和运行时间大多使用 ISO-8601 或上游原始日期的 `TEXT`。

### `journals`

期刊主表：

- `journal_id`：64 位主键
- `library_id`：`scholarly` 或 `cnki`
- `platform_journal_id`：解析后的上游标识
- `title`、`issn`、`eissn`
- `scimago_rank`、`cover_url`
- `available`、`toc_data_approved_and_live`、`has_articles`

### `journal_meta`

保留 CSV 输入和上游解析结果：

| 字段                                   | 语义                                                   |
| -------------------------------------- | ------------------------------------------------------ |
| `journal_id`                           | 主键和 `journals` 外键                                 |
| `source_csv`                           | 来源 CSV 文件名                                        |
| `area`                                 | 项目领域标签                                           |
| `csv_title`、`csv_issn`、`csv_library` | CSV 原值                                               |
| `resolved_source`                      | 实际解析期刊的 `crossref` 或 `openalex`；CNKI 当前为空 |
| `resolved_source_id`                   | 实际 ISSN 或 OpenAlex source ID                        |
| `resolved_title`                       | 解析后的标题                                           |
| `resolved_issn`、`resolved_eissn`      | 解析后的印刷 ISSN 与电子 ISSN                          |

Crossref 对所有 ISSN 返回 404 并触发 OpenAlex fallback 时，resolved 字段记录 OpenAlex 解析结果，而不是覆盖 CSV 审计字段。

### `issues`

- `issue_id`、`journal_id`
- `publication_year`、`title`、`volume`、`number`、`date`
- `is_valid_issue`、`suppressed`、`embargoed`、`within_subscription`

文章缺少可用卷期时可以不关联 issue，并以 `in_press` 表示。

### `articles`

| 分组      | 字段                                                               |
| --------- | ------------------------------------------------------------------ |
| 标识/关系 | `article_id`、`journal_id`、`issue_id`、`platform_id`              |
| 文本      | `title`、`authors`、`abstract`                                     |
| 日期/页码 | `date`、`start_page`、`end_page`                                   |
| 标识符    | `doi`、`pmid`、`retraction_doi`                                    |
| 链接      | `permalink`、`content_location`、`full_text_file`                  |
| 状态      | `suppressed`、`in_press`、`open_access`、`within_library_holdings` |

关键语义：

- scholarly `full_text_file` 是上游 OA PDF/URL；不代表机构订阅。
- CNKI `full_text_file` 保持空，`content_location` 指向详情页。
- `within_library_holdings` 是保留字段，新抓取数据通常为空。
- API 将 `article_id` 和 `journal_id` 序列化为十进制字符串，避免浏览器丢失 64 位整数精度。

### `article_listing`

物化的高频列表字段：

- 文章、期刊和 issue ID
- `publication_year`、`date`、`area`
- OA/in-press/suppressed/holdings 状态
- DOI 和 PMID

当 `listing_state.status=ready` 且表至少有一行时，`/api/articles` 优先使用它；否则回退到关系表联查。

### `article_search`

FTS5 虚表字段：

- `article_id UNINDEXED`
- `title`
- `abstract`
- `doi`
- `authors`
- `journal_title`

建表时若成功加载平台 `simple` 扩展，则使用 `tokenize='simple'`；否则使用默认 FTS5 tokenizer。`CREATE VIRTUAL TABLE IF NOT EXISTS` 不会重建已有 FTS 表，切换 tokenizer 需要显式重建索引。

### 恢复状态

| 表                   | 含义                |
| -------------------- | ------------------- |
| `listing_state`      | 物化列表是否 ready  |
| `journal_year_state` | 某期刊/年份是否完成 |
| `journal_state`      | 某期刊整体是否完成  |

### 索引运行统计

| 表                     | 粒度            | 关键内容                                              |
| ---------------------- | --------------- | ----------------------------------------------------- |
| `index_runs`           | 一次 CSV run    | 状态、期刊总数、成功/失败/恢复计数、错误摘要          |
| `index_path_stats`     | 一个期刊路径    | source/path、works/issues/details/writes、错误        |
| `index_api_call_stats` | source endpoint | logical calls、attempts、状态码、重试、延迟、错误样本 |

错误样本和输出只用于受控诊断，不应写入秘密。

## 认证与业务数据库

### 关系

```text
users
  +-- access_tokens
  +-- cnki_sessions
  +-- folders
  |     +-- favorites
  +-- invite_codes (created_by / used_by)
  +-- notification_settings

scheduled_tasks
  +-- scheduled_task_runs

scheduler_state
scheduler_workers
service_heartbeats
runtime_settings
announcements
```

时间字段大多使用 Rust 生成的 Unix 秒数 `REAL`；`scheduled_for` 是按分钟对齐的 UTC Unix 秒数。

### 用户与认证

#### `users`

`id`、大小写不敏感的唯一 `username`、`password_hash`、`salt`、`is_admin` 和创建/更新时间。

首个管理员由 `admin bootstrap` 在空表上创建；公开注册始终需要邀请码且只创建普通用户。

#### `access_tokens`

`id`、`user_id`、唯一 `token_hash`、`name`、`expires_at`、`created_at`。

`login` 是浏览器会话的保留名称，对应令牌通过 `litradar_session` Cookie 传输；用户创建的 personal tokens 用于外部 Bearer 认证。表没有 token-kind 列：服务以 `name = 'login'` 识别保留行，并在个人令牌列表与配额中排除它。

active personal token 指当前用户拥有、`expires_at > now` 且 `name != 'login'` 的行。令牌新建、列表或验证会按各自操作边界清理过期行；新建个人令牌时，存储层在同一个 `BEGIN IMMEDIATE` 事务内清理、计数并插入，最多接纳 50 行。登录替换则在一个事务内删除并插入保留行，因此并发成功登录最终只保留一个 `login` 行。

配额只约束新接纳，不执行 schema migration，也不重命名、截断或删除已有 personal token。已有账号即使超过 50 行，仍可列出、验证和撤销现有令牌，但必须降到 50 以下才能再创建。历史 `login` 行不会自动分类或改写；它们只会通过正常过期清理、令牌撤销、密码变更/重置或后续登录替换消失。

#### `invite_codes`

`code` 唯一，记录 `created_by`、`used_by`、`used_at` 和 `created_at`。普通用户最多创建一个邀请码；管理员可创建无创建者的后台邀请码。

### CNKI 会话

`cnki_sessions` 每个用户一行：

- `session_json`：非空会话以 `litradarenc:v1:` 保存
- `qr_uuid`
- `status`：`empty`、`waiting_scan`、`active`、`expired` 等
- `token_expires_at`、`created_at`、`updated_at`、`last_used_at`

API 只返回安全派生状态，不返回 token 或 Cookie 值。

### 收藏

`folders`：

- `id`、`user_id`、`name`、`is_tracking`
- 同一用户名称唯一

`favorites`：

- `user_id`、`folder_id`、`article_id`、`db_name`、`note`
- `db_name` 是来源索引文件名，不是 SQLite 外键
- 用户/文件夹/文章/数据库组合唯一

### `notification_settings`

每个用户唯一一行，包含：

- keywords、directions、selected databases
- `folder`/`pushplus` 投递方式和可选文件夹同步
- PushPlus token/template/topic/channel
- 主备 AI base URL/key/model/prompt
- AI 重试次数、启用状态和时间

PushPlus token 与两个 AI key 的非空值加密保存。业务行为见[通知与追踪](../guides/notifications.md)。

### 调度

#### `scheduled_tasks`

- `job_spec`：新任务的类型化 JSON
- `legacy_command`：迁移保留的只读旧命令
- `cron`、`timezone`
- `timeout_seconds`：`1..86400`
- `coalesce`、`enabled`
- 最近状态和时间

`job_spec` 与 `legacy_command` 必须且只能有一个非空；没有 `job_spec` 的行不能启用。

#### `scheduler_state`

固定 `id=1`，`last_checked_at` 单调前进。worker 用它计算最多 24 小时的错过分钟槽。

#### `scheduler_workers`

以 `worker_id` 为主键保存启动和心跳时间。管理接口以 90 秒窗口判断健康；worker 清理超过 7 天的其他陈旧心跳。

#### `scheduled_task_runs`

| 字段                                      | 含义                                                                                  |
| ----------------------------------------- | ------------------------------------------------------------------------------------- |
| `task_id`、`task_name`                    | 原任务 ID 和入队名称快照                                                              |
| `scheduled_for`                           | UTC 计划分钟                                                                          |
| `status`                                  | `pending`、`claimed`、`running`、`success`、`failed`、`timed_out`、`error`、`unknown` |
| `worker_id`、`claim_expires_at`           | 认领和租约                                                                            |
| `claimed_at`、`started_at`、`finished_at` | 生命周期                                                                              |
| `output_summary`                          | 有界内部摘要，不经管理 API 返回                                                       |

`(task_id, scheduled_for)` 唯一，同一任务同时最多一个 claimed/running。未开始的过期认领可回收；已开始但租约过期的运行转为 `unknown`，不会自动重复。

### `service_heartbeats`

主键 `(service, instance_id)`，service 只允许 `api` 或 `worker`。API 每 10 秒更新并在优雅退出时删除自己的行；worker 在调度/任务续租时更新。

`admin backup restore` 在替换前后检查最近 90 秒的该表和旧 worker 心跳，发现活动目标即拒绝。

### `runtime_settings`

`key`、`value`、`updated_at`。只接受七个受管理字段，见[运行配置参考](configuration.md)。

OpenAlex 和 Semantic Scholar key 池的非空值加密保存；数据库没有行时使用代码默认值。运行配置不从进程环境变量回退。

### `announcements`

`id`、`title`、`message`、`priority`、`enabled` 和时间。前台只返回启用公告，按 `high -> normal -> low` 和时间倒序排列。

## 数据库之外的状态

### 变更清单

`data/push_state/<db>.changes.json` 由 `index --update` 生成，是：

- 每周更新
- `notify`
- `push`
- 手动 `push-weekly`

的共同候选输入。

清单中的必填 `db_name` 是数据库的唯一身份；读取方不使用保存的文件系统路径作为回退，也不接受缺少该字段的旧清单。

### 投递状态

- `data/push_state/<db>.json`：notify 和手动 PushPlus
- `data/folder_push_state/<db>.json`：push

状态含 snapshot、当前 run、用户结果和 `delivery_dedupe`，由原子写入维护。

## 备份边界

默认备份只包含 `auth.sqlite`；索引和两个状态目录需要显式选择。部署密钥永远排除。详见[备份与恢复](../operations/backup.md)。
