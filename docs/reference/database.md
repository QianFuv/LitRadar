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
| 认证/业务库 |                     5 |
| 索引库      |                     3 |

连接设置：

| Pragma/设置    | 值       |
| -------------- | -------- |
| `foreign_keys` | `ON`     |
| `journal_mode` | `WAL`    |
| `synchronous`  | `NORMAL` |
| busy timeout   | 30 秒    |

迁移按版本使用独立 `BEGIN IMMEDIATE` 事务，并在同一事务末尾更新 `user_version`。索引 v2 会补齐旧版 `journal_meta` resolved 字段，建立变更事件表，验证投影所依赖的列，并以 1000 行 keyset 批次补回缺失的 `article_search`/`article_listing` 行；文章主表不会因投影修复而删除。索引 v3 新增单行运行租约 `index_run_lease`，迁移不会改写现有运行统计或待发布事件。数据库版本高于当前二进制时，在业务写入前拒绝；不要手工降低版本。

`litradar serve` 在迁移和验证后才绑定端口并进入调度循环。`litradar index` 结束后再次把本次新建的库纳入版本检查。普通 repository 连接不执行 DDL。

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
index_change_events
index_run_lease (最多一行的运行所有权记录)
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

新建索引必须先加载平台 `simple` 扩展，并使用 `tokenize='simple'`。已有声明 `simple` 的 FTS 表会在迁移或写入前执行只读 MATCH 探测；扩展路径或 ABI 错误直接终止且不会静默回退。历史上已经使用默认 tokenizer 的 FTS 表保持原定义，`CREATE VIRTUAL TABLE IF NOT EXISTS` 不会隐式重建它。

### 恢复状态

| 表                   | 含义                |
| -------------------- | ------------------- |
| `listing_state`      | 物化列表是否 ready  |
| `journal_year_state` | 某期刊/年份是否完成 |
| `journal_state`      | 某期刊整体是否完成  |

实时 `--update` 不会把 `journal_state.updated_at` 当作无重叠的精确游标。只有该期刊状态为 `done`、时间可解析且不晚于当前运行时，才取其 UTC 日期并向前重叠 30 天：Crossref 使用 `from-update-date`，OpenAlex fallback 使用 `from_created_date`。缺失、无效或未来时间以及非更新索引都不带日期过滤器，执行完整历史扫描。

增量窗口的每一页复用同一个起始日期。空窗口不会覆盖已有 `journals`/`journal_meta` 或删除文章；所有页面成功后才更新完成时间。中断或失败不会推进旧水位，重试会从同一个 30 天重叠窗口重新读取并依靠幂等写入收敛。

### 索引运行统计

| 表                     | 粒度            | 关键内容                                              |
| ---------------------- | --------------- | ----------------------------------------------------- |
| `index_runs`           | 一次 CSV run    | 状态、期刊总数、成功/失败/恢复计数、错误摘要          |
| `index_path_stats`     | 一个期刊路径    | source/path、works/issues/details/writes、错误        |
| `index_api_call_stats` | source endpoint | logical calls、attempts、状态码、重试、延迟、错误样本 |

实时运行先写入 `running` 父行，再启动期刊 worker。正常完成为 `succeeded`，可控错误为 `failed`；进程失联后由下一次运行回收过期租约时，仍为 `running` 的旧父行改为 `interrupted`。错误样本和输出只用于受控诊断，不应写入秘密。

### `index_run_lease`

索引 v3 用固定 `id=1` 的单行表阻止同一索引数据库被多个实时索引/更新进程并发写入：

- `run_id`：当前 `index_runs` 所有者；该表不使用外键，便于恢复旧运行。
- `heartbeat_at`、`expires_at`：Unix 秒；后台线程每 30 秒续期到未来 300 秒。
- 正常成功或失败只由匹配 `run_id` 的所有者删除租约；无活动写入时表为空。

取得租约、创建 `running` 父行、回收过期所有者以及更新模式下接管待发布事件都在同一个 `BEGIN IMMEDIATE` 事务中。未过期租约会在任何上游请求或 worker 启动前拒绝新运行；过期租约由下一次运行替换，并把旧父行标为 `interrupted`。每个实时写事务和最终清单发布都会再次校验所有权，因此失去租约的旧进程不能继续提交。

### `index_change_events`

索引 v2 引入的磁盘变更账本按 `run_id` 保存标准化的文章 membership 事件：

- `event_type` 只允许 `add` 或 `remove`
- `membership_type` 只允许 `issue` 或 `inpress`
- issue 事件必须带 `issue_id`，in-press 事件必须为空
- `worker_id` 标识可选的并行写入者，`is_backfill` 区分回填事件
- 唯一索引按 run、文章、事件、membership 和期刊/issue 去重

运行顺序、membership 和文章查询均有独立索引。该表是生成外部 changes JSON 的内部持久账本。新的实时 `--update` 会按 `event_id` 以最多 1000 行的批次把所有旧运行待发布事件接管到当前 `run_id`，并应用既有的反向事件抵消和唯一键去重；普通索引不会接管或删除它们。

worker、上游或清单错误会保留当前事件，供下一次更新再次接管。成功路径先原子替换 changes JSON，再在同一最终数据库事务中清理当前事件并完成父行；文件系统重命名与 SQLite 提交之间发生崩溃时允许再次发布，因此该边界是至少一次而不是至多一次。消费者必须继续按清单身份去重，运维人员不得手工把待发布事件改为已发布或直接删除。

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

首个管理员由 `litradar admin bootstrap` 在空表上创建；公开注册始终需要邀请码且只创建普通用户。

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

固定 `id=1`，`last_checked_at` 单调前进。`litradar serve` 的内嵌调度器用它计算最多 24 小时的错过分钟槽。

#### `scheduler_workers`

以 `worker_id` 为主键保存调度实例的启动和心跳时间。字段与表名是持久化调度协议，不表示独立 worker 可执行文件或服务。管理接口与 `/health/ready` 以 90 秒窗口判断健康；内嵌调度器清理超过 7 天的其他陈旧心跳。

#### `scheduled_task_runs`

| 字段                                      | 含义                                                                                               |
| ----------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `task_id`、`task_name`                    | 原任务 ID 和入队名称快照                                                                           |
| `scheduled_for`                           | UTC 计划分钟                                                                                       |
| `status`                                  | `pending`、`claimed`、`running`、`success`、`failed`、`timed_out`、`error`、`unknown`、`cancelled` |
| `worker_id`、`claim_expires_at`           | 认领和租约                                                                                         |
| `claimed_at`、`started_at`、`finished_at` | 生命周期                                                                                           |
| `output_summary`                          | 有界内部摘要，不经管理 API 返回                                                                    |

`(task_id, scheduled_for)` 唯一，同一任务同时最多一个 claimed/running。未开始的过期认领可回收；已开始但租约过期的运行转为 `unknown`，不会自动重复。服务收到终止信号并结束活动子进程时保存 `cancelled`。

### `service_heartbeats`

主键 `(service, instance_id)`，`service` 的存储约束值为 `api` 或 `worker`。统一运行时的 HTTP 组件每 10 秒以内部 `api` 标签更新并在优雅退出时删除自己的行；内嵌调度的实时健康记录位于 `scheduler_workers`。这些标签不是可执行文件或 Compose 服务名。

`litradar admin backup restore` 在替换前后检查最近 90 秒的该表和 `scheduler_workers`，发现活动目标即拒绝。

### `runtime_settings`

`key`、`value`、`updated_at`。只接受七个受管理字段，见[运行配置参考](configuration.md)。

OpenAlex 和 Semantic Scholar key 池的非空值加密保存；数据库没有行时使用代码默认值。运行配置不从进程环境变量回退。

### `announcements`

`id`、`title`、`message`、`priority`、`enabled` 和时间。前台只返回启用公告，按 `high -> normal -> low` 和时间倒序排列。

## 数据库之外的状态

### 变更清单

`data/push_state/<db>.changes.json` 由 `litradar index --update` 生成，是：

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

新建的 v2 备份始终包含 `auth.sqlite` 和完整 `data/meta` 普通文件树；索引和两个状态目录需要显式选择。v1 恢复不会修改目标 Meta 目录，部署密钥始终排除。详见[备份与恢复](../operations/backup.md)。
