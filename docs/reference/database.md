# 数据库参考

LitRadar 把规范内容、可丢弃索引控制状态和用户业务数据放在不同 SQLite 文件中。备份操作见[备份与恢复](../operations/backup.md)，Provider 字段规范见[索引与 Provider 契约](index-provider-contract.md)。

## 文件布局

| 路径                                  |             数量 | 生命周期与责任                                     |
| ------------------------------------- | ---------------: | -------------------------------------------------- |
| `data/index/<catalog>.sqlite`         |     每个目录一个 | 需要备份的 Provider-neutral 内容库                 |
| `data/index-control/<catalog>.sqlite` | 每个活动目录一个 | 可删除的 Provider checkpoint/lease 控制库          |
| `data/auth.sqlite`                    |             一个 | 用户、收藏、会话、配置、任务、公告和受管 Meta 状态 |
| `data/push_state/`                    |        多个 JSON | 变更清单、notify 和手动推送状态                    |
| `data/folder_push_state/`             |        多个 JSON | push 追踪文件夹状态                                |

目录 stem 是内容边界：`data/meta/chinese_journals.csv`、内容库和控制库都使用 `chinese_journals`。Provider 名称不参与文件名。

## 连接和版本

| 数据库      | `PRAGMA user_version` | 升级策略                              |
| ----------- | --------------------: | ------------------------------------- |
| 认证/业务库 |                     6 | 版本化 migration                      |
| 内容索引库  |                     4 | 只创建新库或验证精确 v4；不迁移旧内容 |
| 索引控制库  |                     1 | 可删除后按 v1 重建                    |

可写连接使用 `foreign_keys=ON`、WAL、`synchronous=NORMAL` 和 30 秒 busy timeout。

### 内容库破坏性切换

内容库只接受：

- 不存在的新文件；
- 没有任何 schema object 的空 v0 SQLite；
- 表、列、索引和 `user_version` 精确匹配的 v4。

非空 v0 以及 v1–v3 返回 `IndexRebuildRequired`，文件保持字节不变。未来版本也在业务访问前拒绝。不要手工修改 `user_version` 或拼接表结构。

处理步骤：

1. 停止服务和独立索引命令。
2. 创建并验证备份。
3. 按错误信息移动或删除那个确切的 `data/index/*.sqlite` 文件；系统不会代为删除。
4. 用当前维护目录重新运行 `litradar index`。

旧 v3 文章 ID、收藏和 tracking 引用不会迁移或重映射。

## v4 内容索引库

### 关系

```text
journals (1) ---- (N) issues
   |
   +---- (N) articles ---- (N) article_identity_keys
                |
                +---- article_listing
                +---- article_search (FTS5)

article_change_events (transactional content outbox)
```

内容 schema 只有以下七个内容对象（另有这些对象所需的辅助索引）。Provider 路由、名称、上游 ID、URL、checkpoint、lease、运行统计、Cookie 和会话都不允许出现在该库。

### `journals`

| 字段                                 | 语义                                                   |
| ------------------------------------ | ------------------------------------------------------ |
| `journal_id`                         | 从不可变 `catalog_id` 和 `journal:v1` 生成的 64 位主键 |
| `catalog_id`                         | LitRadar 维护、唯一且 Provider 无关的目录身份          |
| `title`                              | 规范标题                                               |
| `title_aliases_json`                 | 维护标题别名数组                                       |
| `issns_json`                         | 全部规范 ISSN 数组                                     |
| `issn`、`eissn`                      | 首选印刷/电子 ISSN                                     |
| `area`                               | 维护领域                                               |
| `utd_*`、`abs_*`、`fms_*`、`fmscn_*` | 维护排名字段                                           |

### `issues`

字段为 `issue_id`、`journal_id`、`publication_year`、`title`、`volume`、`number` 和 `date`。`issue_id` 只使用规范出版身份，不使用 Provider issue ID。

文章可以没有 `issue_id`，例如上游只能确认 in-press 内容时。

### `articles`

| 分组         | 字段                                                 |
| ------------ | ---------------------------------------------------- |
| 关系/身份    | `article_id`、`journal_id`、可空 `issue_id`          |
| 内容         | `title`、`authors_json`、`abstract_text`             |
| 出版         | `publication_year`、`date`、`start_page`、`end_page` |
| 外部规范标识 | `doi`、`pmid`、`retraction_doi`                      |
| 内容状态     | 可空布尔 `open_access`、`in_press`                   |

没有 `platform_id`、`permalink`、`content_location`、`full_text_file`、Provider/source、馆藏或订阅列。API 把 64 位 article/journal ID 序列化为十进制字符串，避免 JavaScript 精度损失。

### `article_identity_keys`

主键为 `(identity_kind, identity_value)`，kind 只允许：

- `doi`；
- `pmid`；
- `bibliographic`。

每个 alias 指向一个不可变 `article_id`，同一文章可以拥有多个不同的 DOI alias。写入新 batch 前，writer 同时查询所有 alias：零命中时按最强 alias 生成新 ID；一个 ID 命中时复用；多个 ID 命中时明确报冲突。

当同一已解析文章出现不同 DOI 时，writer 保存输入与合并结果的全部 DOI alias，并把规范 DOI 的字典序最小值写入单值 `articles.doi` 及其列表/FTS 投影。PMID 和撤稿 DOI 冲突仍会中止事务；已有 alias 不会删除、改绑或分配新的 article ID。

### `article_listing`

物化高频筛选字段：文章、期刊、issue ID，出版年份/日期，OA/in-press，DOI、PMID 和领域。`/api/articles` 的过滤、计数和游标分页基于该表。

### `article_search`

FTS5 使用内置 `unicode61 remove_diacritics 2`，字段为：

- `article_id UNINDEXED`；
- `title`；
- `abstract_text`；
- `doi`；
- `pmid`；
- `authors`；
- `journal_title`。

内容 v4 不再依赖外部 `simple` tokenizer 创建 schema。

### `article_change_events`

这是和内容写入同事务的 Provider-neutral outbox：

- `content_revision` 由索引核心生成；
- `change_kind` 只允许 `upsert` 或 `remove`；
- 记录 article/journal/issue 和 in-press membership；
- revision 唯一索引让 Provider 重试和控制状态丢失重放幂等收敛。

`--update` 把事件生成到 `data/push_state/<db>.changes.json`。文件发布成功后清理已发布 outbox；文件系统替换和 SQLite 提交之间仍是至少一次边界，消费者继续按身份去重。

## v1 索引控制库

控制库位于 `data/index-control`，与内容发现、REST 查询和备份完全分离。

### `provider_leases`

主键 `(catalog_name, provider_name)`，保存 `run_id`、`heartbeat_at`、`expires_at`。父进程每 30 秒续期；未过期所有者阻止同一目录/Provider 的并发运行，过期 lease 可被后续运行接管。

### `provider_checkpoints`

主键 `(catalog_name, provider_name, scope_kind, scope_key)`。scope 只允许 `listing`、`journal`、`year`；`checkpoint` 是 Provider 私有 opaque 文本。

切换 Provider 会自然使用新的 checkpoint namespace，而不修改内容库。删除或丢失控制库后，下一次运行从头抓取并通过 `article_identity_keys` 和 upsert 规则收敛。控制库不需要迁移、恢复或备份。

## 认证与业务数据库

### 关系

```text
users
  +-- access_tokens
  +-- cnki_sessions
  +-- folders -- favorites
  +-- invite_codes
  +-- notification_settings

scheduled_tasks -- scheduled_task_runs
scheduler_state
scheduler_workers
service_heartbeats
runtime_settings
managed_meta_catalogs
announcements
```

时间字段大多是 Rust 生成的 Unix 秒数 `REAL`；`scheduled_for` 是按分钟对齐的 UTC Unix 秒数。

### 用户、令牌和邀请码

`users` 保存大小写不敏感的唯一用户名、密码 hash/salt、管理员标记和时间。首个管理员只能通过 `litradar admin bootstrap` 创建；公开注册需要邀请码。

`access_tokens` 保存唯一 token hash，不保存明文 token。`name='login'` 是浏览器 Cookie 会话的保留行；其他 active personal token 每用户最多 50 个。达到上限只阻止新建，不删除历史行。

`invite_codes` 记录创建、使用者和时间；普通用户最多创建一个邀请码。

### CNKI 会话

`cnki_sessions` 每个用户一行，非空 `session_json` 使用 `litradarenc:v1:` 密文。其余字段包括 `qr_uuid`、status、过期和创建/更新时间。API 只返回安全派生状态，不返回 token 或 Cookie。

在线文章全文动作可以读取当前用户已有的 active 会话，但不会更新 `session_json`、`updated_at` 或 `last_used_at`。

### 收藏

`folders` 以用户和名称唯一；`favorites` 保存 `user_id`、`folder_id`、稳定 `article_id`、内容库 `db_name`、note 和时间。`db_name` 是内容库文件名，不是 Provider 或 SQLite 外键。

v4 破坏性重建不会重映射旧 favorite 的 article ID；无法解析的旧引用由运维人员或用户清理。

### 用户通知配置

`notification_settings` 每用户一行，保存数据库、关键词、方向、投递方式、PushPlus 和主备 AI 配置。PushPlus token 与 AI key 加密。业务语义见[通知与追踪](../guides/notifications.md)。

### 调度和活动门禁

- `scheduled_tasks` 保存类型化 `job_spec`；旧 `legacy_command` 只读且不能启用。
- `scheduled_task_runs` 保存认领、运行、取消、超时和终态。
- `scheduler_state` 保存单调调度游标。
- `scheduler_workers` 保存内嵌调度心跳。
- `service_heartbeats` 保存统一进程 HTTP 组件的活动记录。

`litradar admin backup restore` 在替换前后检查最近 90 秒的心跳，目标仍活动时拒绝恢复。

### `runtime_settings`

只接受[运行配置](configuration.md)列出的 11 个字段。两个 key pool 的非空值加密；Provider 路由和顺序是非秘密运行配置，不进入内容库。

### `managed_meta_catalogs` 和 `announcements`

`managed_meta_catalogs` 记录官方 bundle 版本/hash 所有权，用于保护用户修改的目录。`announcements` 保存标题、消息、优先级、启用状态和时间。

## 数据库之外的状态

### 变更与投递状态

- `data/push_state/<db>.changes.json`：索引 update 的 Provider-neutral 变更清单；
- `data/push_state/<db>.json`：notify/手动 PushPlus 状态；
- `data/folder_push_state/<db>.json`：push 状态。

清单的 `db_name` 是目标内容库身份；读取方不使用历史文件系统路径或 Provider 名称回退。

## 备份边界

v2 备份固定包含 `auth.sqlite` 和完整 `data/meta`。`--include-indexes` 只包含 `data/index/*.sqlite` 内容库；`data/index-control` 永远排除。push 状态需要 `--include-push-state`。部署密钥始终单独保存。
