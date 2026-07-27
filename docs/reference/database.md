# 数据库参考

LitRadar 把规范内容、可丢弃索引控制状态和用户业务数据放在不同 SQLite 文件中。备份操作见[备份与恢复](../operations/backup.md)，Provider 字段规范见[索引与 Provider 契约](index-provider-contract.md)。

## 文件布局

| 路径                                  |             数量 | 生命周期与责任                                     |
| ------------------------------------- | ---------------: | -------------------------------------------------- |
| `data/index/<catalog>.sqlite`         |     每个目录一个 | 需要备份的 Provider-neutral 内容库                 |
| `data/index-control/<catalog>.sqlite` | 每个活动目录一个 | 可删除的 Provider checkpoint/lease 控制库          |
| `data/auth.sqlite`                    |             一个 | 用户、收藏、会话、配置、任务、公告、审计、投递状态和受管 Meta 状态 |
| `data/push_state/`                    |        多个 JSON | Provider-neutral 变更清单和保留的旧 notify 导入源  |
| `data/folder_push_state/`             |        多个 JSON | 保留的旧 push 导入源                               |

目录 stem 是内容边界：`data/meta/chinese_journals.csv`、内容库和控制库都使用 `chinese_journals`。Provider 名称不参与文件名。

## 连接和版本

| 数据库      | `PRAGMA user_version` | 升级策略                              |
| ----------- | --------------------: | ------------------------------------- |
| 认证/业务库 |                    10 | 版本化 migration                      |
| 内容索引库  |                     6 | 新建/验证精确 v6；精确 v4/v5 原子迁移到 v6 |
| 索引控制库  |                     1 | 可删除后按 v1 重建                    |

可写连接使用 `foreign_keys=ON`、WAL、`synchronous=NORMAL` 和 30 秒 busy timeout。

### 内容库破坏性切换

内容库只接受：

- 不存在的新文件；
- 没有任何 schema object 的空 v0 SQLite；
- 表、列、索引和 `user_version` 精确匹配的 v4 或 v5，随后在一个事务内迁移到 v6；
- 表、列、索引和 `user_version` 精确匹配的 v6。

非空 v0 以及 v1–v3 返回 `IndexRebuildRequired`，文件保持字节不变。未来版本也在业务访问前拒绝。v4 会先新增期刊身份键，再与 v5 一样迁移到 v6。迁移保留期刊、期次、文章、投影、outbox、身份键和稳定 ID；唯一有意丢弃的是旧 `articles.retraction_doi`，因为旧适配器无法区分真正撤稿与通用 Crossref relation。不要手工修改 `user_version` 或拼接表结构。

处理步骤：

1. 停止服务和独立索引命令。
2. 创建并验证备份。
3. 按错误信息移动或删除那个确切的 `data/index/*.sqlite` 文件；系统不会代为删除。
4. 用当前维护目录重新运行 `litradar index`。

旧 v1–v3 文章 ID、收藏和 tracking 引用不会迁移或重映射；精确 v4/v5 到 v6 的迁移不重映射这些 ID。

## v6 内容索引库

### 关系

```text
journal_identity_keys ---- canonical catalog identity ownership

journals (1) ---- (N) issues
   |
   +---- (N) articles ---- (N) article_identity_keys
                |
                +---- (N) article_retraction_dois
                +---- article_listing
                +---- article_search (FTS5)

article_change_events (transactional content outbox)
```

内容 schema 只有以下九个内容对象（另有这些对象所需的辅助索引）。Provider 路由、名称、上游 ID、URL、checkpoint、lease、运行统计、Cookie 和会话都不允许出现在该库。

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

### `journal_identity_keys`

主键为 `(identity_kind, identity_value)`；kind 只允许 `catalog_id` 和 `issn`，每个键指向一个 `canonical_catalog_id`。一个规范期刊拥有：

- 当前 `catalog_id`；
- `catalog_aliases` 中所有已退役 catalog ID；
- `all_issns` 中所有规范 ISSN。

该表故意不使用到 `journals` 的外键，因此空内容库可以先登记完整身份所有权，而不创建未索引的 journal 壳。每次索引在 Provider 分配和请求之前进行目录级事务归并：保留无关历史键、登记当前目录键、刷新已有规范 journal 的维护元数据及 listing/FTS 投影，并且只删除没有 issue、article、listing 或 outbox 历史的旧 alias journal 壳。旧壳仍有内容、身份键属于另一规范期刊，或旧 alias 仍有任何 Provider namespace 的 checkpoint 时，运行在 Provider 请求前固定失败且不做部分归并。

每个内容 batch 的写事务还会重新核对 catalog ID、catalog alias、ISSN 和确定性 `journal_id` 的所有权，防止预检后出现错误改绑。身份键只能新增或由已证明为空的旧 alias 壳释放；不会猜测合并两个已有期刊实体。

### `issues`

字段为 `issue_id`、`journal_id`、`publication_year`、`title`、`volume`、`number` 和 `date`。`issue_id` 只使用规范出版身份，不使用 Provider issue ID。

文章可以没有 `issue_id`，例如上游只能确认 in-press 内容时。

### `articles`

| 分组         | 字段                                                 |
| ------------ | ---------------------------------------------------- |
| 关系/身份    | `article_id`、`journal_id`、可空 `issue_id`          |
| 内容         | `title`、`authors_json`、`abstract_text`             |
| 出版         | `publication_year`、`date`、`start_page`、`end_page` |
| 外部规范标识 | `doi`、`pmid`                                       |
| 内容状态     | 可空布尔 `open_access`、`in_press`                   |

没有 `platform_id`、`permalink`、`content_location`、`full_text_file`、Provider/source、馆藏或订阅列。API 把 64 位 article/journal ID 序列化为十进制字符串，避免 JavaScript 精度损失。

### `article_identity_keys`

主键为 `(identity_kind, identity_value)`，kind 只允许：

- `doi`；
- `pmid`；
- `bibliographic`。

每个 alias 指向一个不可变 `article_id`，同一文章可以拥有多个不同的 DOI alias。写入新 batch 前，writer 同时查询所有 alias：零命中时按最强 alias 生成新 ID；一个 ID 命中时复用；多个 ID 命中时明确报冲突。

当同一已解析文章出现不同 DOI 时，writer 保存输入与合并结果的全部 DOI alias，并把规范 DOI 的字典序最小值写入单值 `articles.doi` 及其列表/FTS 投影。PMID 冲突仍会中止事务；已有 alias 不会删除、改绑或分配新的 article ID。

### `article_retraction_dois`

主键为 `(article_id, retraction_doi)`，并通过级联外键关联 `articles`。writer 对同一规范文章观察到的全部撤稿 DOI 做集合并集，再事务性替换关联行；读取时始终按 DOI 字典序返回非空数组 `retraction_dois`，没有撤稿记录时返回空数组。该表不保存 Provider、source、URL、更新时间或原始 payload。

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

内容 v6 不依赖外部 `simple` tokenizer 创建 schema。

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
  +-- invite_codes -- invite_code_uses
  +-- notification_settings

scheduled_tasks -- scheduled_task_runs
scheduler_state
scheduler_workers
service_heartbeats
runtime_settings
managed_meta_catalogs
announcements
security_audit_events -- security_audit_maintenance

delivery_checkpoints
delivery_runs -- delivery_run_items
      |
      +-- delivery_dedupe
      +-- delivery_leases
```

时间字段大多是 Rust 生成的 Unix 秒数 `REAL`；`scheduled_for` 是按分钟对齐的 UTC Unix 秒数。

### 用户、令牌和邀请码

`users` 保存大小写不敏感的唯一用户名、密码 hash/salt、管理员标记和时间。新密码的 `password_hash` 是包含算法、版本、成本、salt 和输出的 `$argon2id$` PHC 字符串，旧 `salt` 列为空；旧 PBKDF2 hex 行保留原 salt，并在正确登录后通过原 hash+salt 的 CAS 单次升级。首个管理员只能通过 `litradar admin bootstrap` 创建；公开注册需要邀请码。密码变更与该用户全部 `access_tokens` 的删除在同一个 immediate transaction 中提交，目标用户不存在时返回未更新，删除令牌失败时密码写入回滚。管理员标记更新和用户删除也使用 actor-aware immediate transaction：事务内重新验证 actor、target 和管理员计数，任何成功提交都至少保留一个 `is_admin = 1` 行。

`access_tokens` 保存唯一 token hash，不保存明文 token。`name='login'` 是浏览器 Cookie 会话的保留行；其他 active personal token 每用户最多 50 个。达到上限只阻止新建，不删除历史行。所有读取路径统一以 `expires_at <= now` 判定过期并清理；salt、原始 token 和邀请码由操作系统 CSPRNG 生成。

认证库 v12 的 `invite_codes` 除兼容字段 `used_by`/`used_at` 外，还保存 `expires_at`、不可逆 `revoked_at`、`max_uses` 和 `use_count`。`invite_code_uses` 为每次已提交兑换保存邀请码、用户和时间；删除用户只把历史中的用户引用设为 `NULL`，不能删除邀请码历史。`used_by`/`used_at` 表示首位兑换者，仅为旧客户端兼容。

普通用户由部分唯一索引保证最多一个 `revoked_at IS NULL` 的发行记录；过期或用尽后可以通过 rotate 原子撤销旧记录并签发新码。注册在 `BEGIN IMMEDIATE` 事务中以 `revoked_at IS NULL AND expires_at > now AND use_count < max_uses` 条件递增计数，同时插入用户、兑换历史、默认文件夹和审计事件，因此并发争用最后一个名额时最多一个事务成功。管理员生成的 `created_by IS NULL` 邀请码不受单发行索引限制，但有效期最多 365 天且用量上限为 1000。

v11 升级保留旧 ID、code、创建者、首位使用者和使用时间；已使用行补入一条 `invite_code_uses` 历史，同一普通创建者的旧重复行只保留最高 ID 为未撤销状态。迁移后的旧码至少获得迁移时起 7 天的兼容有效期。

### CNKI 会话

`cnki_sessions` 每个用户一行，非空 `session_json` 使用 `litradarenc:v1:` 密文。其余字段包括 `qr_uuid`、status、过期和创建/更新时间。API 只返回安全派生状态，不返回 token 或 Cookie。

在线文章全文动作可以读取当前用户已有的 active 会话，但不会更新 `session_json`、`updated_at` 或 `last_used_at`。

### 收藏

`folders` 以用户和名称唯一；auth schema v11 还通过 `idx_folders_one_tracking_per_user` 部分唯一索引保证每个用户最多一个 `is_tracking = 1` 文件夹。v10 升级时若发现多个旧 tracking folder，会按最低 folder ID 保留一个并在同一迁移事务内清除其余标记。`favorites` 保存 `user_id`、`folder_id`、稳定 `article_id`、内容库 `db_name`、note 和时间。`db_name` 是内容库文件名，不是 Provider 或 SQLite 外键。

创建 tracking folder、切换 tracking folder、单条幂等收藏以及批量添加/删除/移动都使用 `BEGIN IMMEDIATE`。重复收藏由 `ON CONFLICT DO NOTHING RETURNING id` 与精确既有行查询返回真实 ID；批量中途失败会整体回滚。动态 favorite `IN` 查询每块最多 500 个 ID。

v1–v3 的破坏性重建不会重映射旧 favorite 的 article ID；精确 v4/v5 到 v6 的迁移保留 ID。无法解析的旧引用由运维人员或用户清理。

### 用户通知配置

`notification_settings` 每用户一行，保存数据库、关键词、方向、投递方式、PushPlus 和主备 AI 配置。PushPlus token 与 AI key 加密。业务语义见[通知与追踪](../guides/notifications.md)。

### 持久投递状态

认证库 v10 新增五张投递表，所有应用自有状态都受 `CHECK` 约束并使用类型化枚举读取：

- `delivery_checkpoints` 以 `(workflow, db_name)` 唯一，保存规范 snapshot、最后完成时间、旧状态导入 hash 和单调 `revision`；
- `delivery_runs` 保存外部任务 ID、触发来源、模式、用户、deadline、取消位、owner lease、终态和 `revision`；同一 `(workflow, db_name)` 只能有一个 active run，同一用户只能有一个 queued/active manual run；
- `delivery_run_items` 以 `(delivery_run_id, item_kind, item_key)` 唯一，区分 `pending`、可过期接管的 `claimed`、不可自动重放的 `sending` 及终态；
- `delivery_dedupe` 以 `(workflow, db_name, user_id, article_id)` 唯一，在外部副作用前建立 `reserved` 行，再明确落为 `confirmed` 或 `unknown`；
- `delivery_leases` 以 `(workflow, db_name)` 唯一，释放时保留行并递增 `revision`，防止删除重建造成 ABA。

run、item、checkpoint 和 lease 的变更都使用 owner/revision compare-and-swap。run 终态、checkpoint CAS 和 workflow lease 释放在一个 transaction 中提交；一次 subscriber 外部尝试的 item 终态和全部文章 dedupe 也在一个 transaction 中提交。只有租约已过期的 run、pre-send item 或 workflow lease 可被新 owner 接管；接管时 `claimed` item 回到 pending 并释放 pre-send reservation，`sending` item 与对应 reservation 则固定收敛为 `unknown`，不得自动重新投递。

启动迁移会先读取 `data/push_state/<db>.json` 和 `data/folder_push_state/<db>.json`，校验所有文件后再用一个 immediate transaction 导入。每个源文件的 SHA-256 保存在 checkpoint 中：相同 hash 重启时跳过，已导入文件内容变化则拒绝启动，损坏文件使整批零写入。源文件和 `.changes.json` 都不会被导入器删除；未知旧状态只映射为固定 `unknown`/`unrecognized` 分类，不把原始状态或错误内容写入数据库。

### 调度和活动门禁

- `scheduled_tasks` 保存类型化 `job_spec`；旧 `legacy_command` 只读且不能启用。
- `scheduled_task_runs` 保存认领、运行、取消、超时和终态。
- `scheduler_state` 保存单调调度游标。
- `scheduler_workers` 保存内嵌调度心跳。
- `service_heartbeats` 保存统一进程 HTTP 组件的活动记录。

`litradar admin backup restore` 在替换前后检查最近 90 秒的心跳，目标仍活动时拒绝恢复。

### 持久安全审计

认证库 v9 新增 `security_audit_events`。每行包含可空 `actor_id`/`target_id`、固定 `action`/`outcome`/`reason`、服务器 `request_id`、限流 `source_class`/`bucket`/计数以及 `occurred_at`。这些字段只保存分类和内部 ID，不保存用户名、密码、原始 token、邀请码、IP、Header 或请求内容。

索引覆盖 `(occurred_at, id)`、`(action, outcome, occurred_at)`、`(actor_id, occurred_at)` 和非空 `request_id`。触发器拒绝 `UPDATE`，从而保持追加语义；受控保留任务仍可 `DELETE`。安全变更 repository 在业务事务提交前插入必需审计行，插入失败会回滚业务变更。认证拒绝与限流通过独立 immediate transaction 同步追加。

`security_audit_maintenance` 只包含 `id=1` 和可空 `last_retention_at`。保留任务在同一 immediate transaction 中检查/更新该时间并删除最多 10,000 条过期记录，因此失败不会留下部分删除或错误推进窗口。默认保留 180 天，配置范围为 1–3650 天。

### `runtime_settings`

只接受[运行配置](configuration.md)列出的 17 个字段。两个 key pool 的非空值加密；Provider 路由、顺序和审计保留天数是非秘密运行配置，不进入内容库。

### `managed_meta_catalogs` 和 `announcements`

`managed_meta_catalogs` 记录官方 bundle 版本/hash 所有权，用于保护用户修改的目录。`announcements` 保存标题、消息、优先级、启用状态和时间。

## 数据库之外的状态

### 变更与投递状态

- `data/push_state/<db>.changes.json`：索引 update 的 Provider-neutral 变更清单；
- `data/push_state/<db>.json`：旧 notify/手动 PushPlus 状态导入源；
- `data/folder_push_state/<db>.json`：旧 push 状态导入源。

可变 checkpoint、run、item、dedupe 和 lease 的唯一权威存储是 `auth.sqlite`。运行时不再创建 `<db>.json` 或固定 `.tmp`，也不接受状态目录参数。清单的 `db_name` 是目标内容库身份；读取方不使用历史文件系统路径或 Provider 名称回退。旧导入源会原样保留，不能在导入后手工修改；`.changes.json` 始终保持文件契约，不进入认证库。

## 备份边界

v2 备份固定包含 `auth.sqlite` 和完整 `data/meta`，因此持久投递状态总在认证库快照中。`--include-indexes` 只包含 `data/index/*.sqlite` 内容库；`data/index-control` 永远排除。Provider-neutral `.changes.json` 和保留的旧导入源需要 `--include-push-state`。部署密钥始终单独保存。
