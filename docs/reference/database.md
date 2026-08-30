# 数据库参考

LitRadar 把规范内容、可丢弃索引控制状态和用户业务数据放在不同 SQLite 文件中。备份操作见[备份与恢复](../operations/backup.md)，Provider 字段规范见[索引与 Provider 契约](index-provider-contract.md)。

## 文件布局

| 路径                                      |             数量 | 生命周期与责任                                                     |
| ----------------------------------------- | ---------------: | ------------------------------------------------------------------ |
| `data/index/<catalog>.sqlite`             |     每个目录一个 | 需要备份的 Provider-neutral 内容库                                 |
| `data/index-control/index-batches.sqlite` |         项目一个 | 可删除的 batch/catalog phase、manifest intent 和全局 lease ledger  |
| `data/index-control/<catalog>.sqlite`     | 每个活动目录一个 | 可删除的 v4 Provider anchor/run checkpoint/lease 控制库            |
| `data/index-work/scholarly/`              | 每个遍历一个工作集 | 可丢弃的 Crossref SQLite、归属记录及事务文件，不是内容库或控制库 |
| `data/auth.sqlite`                        |             一个 | 用户、收藏、会话、配置、任务、公告、审计、投递状态和受管 Meta 状态 |
| `data/push_state/`                        |        多个 JSON | Provider-neutral 变更清单和保留的旧 notify 导入源                  |
| `data/folder_push_state/`                 |        多个 JSON | 保留的旧 push 导入源                                               |

目录 stem 是内容边界：`data/meta/chinese_journals.csv`、内容库和控制库都使用 `chinese_journals`。Provider 名称不参与文件名。

## 连接和版本

| 数据库             | `PRAGMA user_version` | 升级策略                                              |
| ------------------ | --------------------: | ----------------------------------------------------- |
| 认证/业务库        |                    15 | 版本化 migration                                      |
| 内容索引库         |                     6 | 新建/验证精确 v6；精确 v4/v5 原子迁移到 v6            |
| 项目 batch ledger  |                     2 | 新建/验证精确 v2；精确 v1 原位迁移到 v2；可删除后重建 |
| catalog 索引控制库 |                     4 | v0/v1/v2/v3 安全事务迁移；可删除后按 v4 重建          |

上述内容、业务和控制库的可写连接使用 `foreign_keys=ON`、WAL、`synchronous=NORMAL` 和 30 秒 busy timeout。Crossref 私有工作集使用下面单独说明的连接策略，不改变这些正式 schema 版本。

### Crossref 可丢弃工作集

`data/index-work/scholarly/<token>.sqlite` 使用独立私有格式 v1，配套归属 JSON 和 SQLite 事务文件。token 是随机 32 位小写 hex，根目录由 core 构造，通过 private worker protocol v8 的 Scholarly bootstrap 传入，不能由 traversal 指定任意路径。文件保存完整创建日期分片树、必要的 Crossref 书目字段、去重/排序索引、计数及冻结状态；不存凭据、资源 URL 或 references。归属和路径不符、symlink/reparse point、foreign schema 均在使用或清理前拒绝。

工作集使用 `foreign_keys=ON`、DELETE journal、`synchronous=FULL`、5 秒 busy timeout、4 MiB page cache 和禁用 mmap；4 KiB 页及 `max_page_count` 把主文件限制为 4 GiB，事务日志需要额外空间。4 MiB 不是进程总内存上限。记录与当前分片进度在同一事务中保存，完整计数通过后通过索引按 keyset 读取，每页最多 225 条、16 MiB；不将全刊读入内存，也不使用 `/tmp` 大排序文件。

成功 anchor v1、Scholarly traversal v2 和工作集格式 v1 是三个不同的版本。严格读取有效 traversal v1：旧 Crossref cursor 从新查询头重放，保留原 base/candidate；OpenAlex 保留旧 cursor 语义。NULL traversal 正常新建，不需要手工修复 SQLite；旧二进制不能直接恢复 v2。

core 仍以内容事务和控制事务确认进度。工作集超前一页时重放该步骤；缺失或可识别损坏时从相同 `C/T`、update 条件和 candidate 重新收集，不能续用缺少前半记录的旧 cursor。Provider 完成输出后可清理缓存，但后续控制提交失败仍需安全重建并幂等重放。磁盘满或容量超限保留正式内容与旧 anchor，不截断结果。正常失败保留未完成缓存；不会自动清理未知文件或其他期刊缓存。

该目录被 Git 忽略，内容发现、备份和恢复均不使用它。丢失工作集只影响重取成本，不要求删除 `data/index` 或 `data/index-control`。计数和冻结范围并不保证远端快照，查询与期次覆盖语义见 [Scholarly](sources/scholarly.md)。

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

该表故意不使用到 `journals` 的外键，因此空内容库可以先登记完整身份所有权，而不创建未索引的 journal 壳。每次索引在 Provider 分配和请求之前进行目录级事务归并：保留无关历史键、登记当前目录键、刷新已有规范 journal 的维护元数据及 listing/FTS 投影，并且只删除没有 issue、article、listing 或 outbox 历史的旧 alias journal 壳。旧壳仍有内容、身份键属于另一规范期刊，或旧 alias 仍有任何 Provider namespace 的 anchor/run 状态时，运行在 Provider 请求前固定失败且不做部分归并。

每个内容 batch 的写事务还会重新核对 catalog ID、catalog alias、ISSN 和确定性 `journal_id` 的所有权，防止预检后出现错误改绑。身份键只能新增或由已证明为空的旧 alias 壳释放；不会猜测合并两个已有期刊实体。

### `issues`

字段为 `issue_id`、`journal_id`、`publication_year`、`title`、`volume`、`number` 和 `date`。`date` 原样保存真实的 `YYYY`、`YYYY-MM` 或 `YYYY-MM-DD` 精度；API 从该值派生 `date_precision`，不新增冗余列。`issue_id` 只使用规范出版身份，不使用 Provider issue ID。

文章可以没有 `issue_id`，例如上游只能确认 in-press 内容时。

### `articles`

| 分组         | 字段                                                 |
| ------------ | ---------------------------------------------------- |
| 关系/身份    | `article_id`、`journal_id`、可空 `issue_id`          |
| 内容         | `title`、`authors_json`、`abstract_text`             |
| 出版         | `publication_year`、`date`、`start_page`、`end_page` |
| 外部规范标识 | `doi`、`pmid`                                        |
| 内容状态     | 可空布尔 `open_access`、`in_press`                   |

没有 `platform_id`、`permalink`、`content_location`、`full_text_file`、Provider/source、馆藏或订阅列。API 把 64 位 article/journal ID 序列化为十进制字符串，避免 JavaScript 精度损失。

Provider 写入前使用真实 Gregorian 日历校验日期，年/月信息不会被补成虚假的月/日。旧版本已经写入的 `YYYY-01-01` 无法与真实 1 月 1 日可靠区分，因此迁移不会猜测降级；重新索引获得原始精度后才会自然纠正。其他无法通过日历校验的历史文本仍可读取，但 API 的 `date_precision` 为 `null`。

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

`--update` 把事件生成到 `data/push_state/<db>.changes.json`。项目 batch ledger 先持久化精确 JSON 字节和 inclusive through-event cursor，再原子替换文件、幂等删除 `event_id <= cursor` 的行并推进 catalog phase。只要 active batch ledger 保留，rename 或 acknowledgement 任一侧崩溃都会重放同一 payload；ledger 丢失后文件系统与 SQLite 仍是至少一次边界，消费者继续按身份去重。空 outbox 不会覆盖已有的有界、可解析且 `db_name` 匹配的 manifest。Bootstrap 和 `--full-rescan` 不发布清单，并在成功结束时丢弃本次无需投递的 outbox。

## v2 项目 batch ledger

`data/index-control/index-batches.sqlite` 每个项目只有一个，负责跨 CSV 的恢复顺序和唯一 active invocation。它包含：

- `index_batches`：batch ID、`active/abandoning/completed/abandoned` 状态、兼容性 fingerprint、selection、sync mode、遗留 issue-batch 恢复值、notify flags 和时间；部分唯一索引保证最多一个 active/abandoning batch。
- `index_batch_catalogs`：稳定 ordinal、CSV basename/stem/摘要、Provider route、journal count、phase、安全 outcome 计数、精确 manifest intent，以及可空的 notify attempt ID、typed status、exit code、最近确认的 Unknown attempt ID/时间。
- `index_batch_lease`：固定单行全局 lease，保存 batch、owner、heartbeat 和 expiry。

catalog phase 只允许以下前向路径：

```text
pending -> indexing -> completed
                    -> manifest_prepared -> manifest_published -> completed
                                                           \-> notifying -> completed
```

fingerprint 包含 CSV selection、顺序和精确内容、Provider route、sync mode、遗留 issue-batch 恢复值与 notify flags；不包含 workers/processes、timeout、代理或凭据。`issue_batch_size` 列和对应 fingerprint 字段保留 v1/v2 ledger 的 active-batch 匹配语义，但当前 Provider 不读取它来控制运行时分批、并发或内存。默认 resume 只重新打开兼容 active batch。成功 batch 保持 completed 历史且下一条命令创建新 batch，所以历史 completed row 不会使下一次 update 永久跳过 journal。

notify status 只允许 `running/idle/completed/skipped/failed/cancelled/timed_out/unknown`。父进程在 child 启动前把 attempt 写为 Running，结果以当前 attempt ID 做 CAS；只有 `idle/completed/skipped` 且 exit code 为 0 时 `notifying` catalog 才能进入 completed。Unknown acknowledgement 的 ID 与时间必须成对出现，并与新 Running attempt 在同一 immediate transaction 中写入。v1 ledger 原位迁移；active Notifying 无论旧 exit code 为何都转为带 legacy attempt ID 的 Unknown，防止未经 typed protocol 证明就自动重发。

`--no-resume` 先把 active batch 置为 abandoning；调用方从各 catalog v4 控制库删除仅属于该 batch 的 run checkpoint 后，事务性标记旧 batch abandoned 并创建 replacement。committed anchor、内容和 outbox 不属于清理范围。若未完成 catalog 已有 `outcome_manifest_path` 且原 batch 启用了 notify，admission 会拒绝 abandonment，避免把已经发布或可能已发布的 handoff 静默丢弃；必须先恢复原 batch。

## v4 catalog 索引控制库

控制库位于 `data/index-control`，与内容发现、REST 查询和备份完全分离。所有键都包含 `catalog_name`、`provider_name` 和规范 `catalog_id`；opaque 值非空时最多 65,536 字节，核心从不解析其 Provider 私有结构。

### `provider_leases`

主键 `(catalog_name, provider_name)`，保存 `run_id`、`heartbeat_at`、`expires_at`。父进程每 30 秒续期；未过期所有者阻止同一目录/Provider 的并发运行，过期 lease 可被后续运行接管。

### `provider_sync_anchors`

主键 `(catalog_name, provider_name, catalog_id)`，保存可空 `committed_anchor`、`completed_at` 和可空 `completed_batch_id`。该行只在整本期刊运行 Complete 后创建或替换：

- 行不存在：没有可信成功状态；下一次运行必须完整覆盖。
- 行存在且 `completed_batch_id` 等于 active batch：默认 resume 在三种模式中都可零请求跳过该 journal。
- 行属于更早 batch 且 `committed_anchor IS NULL`：该 Provider 没有可复用增量边界；新 batch 不会跳过，Bootstrap/Incremental 都安全完整扫描。
- 行属于更早 batch 且 anchor 非空：新 Incremental batch 把它作为本次冻结 base 原样交给同一 Provider；它是边界，不是 skip marker。

内容库“看起来最新”的期次不会用于重算该值。Continue 不能修改 committed anchor；只有最终内容批次已经提交后，Complete 才推进它。

### `provider_run_checkpoints`

主键同样是 `(catalog_name, provider_name, catalog_id)`。每行保存可空 legacy `batch_id`、当前 `run_id`、`sync_mode`（`bootstrap` / `incremental` / `full_rescan`）、冻结的可空 `base_anchor`、可空 `traversal_checkpoint`、`started_at` 和 `updated_at`。

`base_anchor` 在运行期间不变；`traversal_checkpoint` 是 Provider 私有的页码、cursor、期次位置或组合状态。resume 只接管 batch、mode 与 base 都匹配的行；foreign/legacy batch、模式不匹配或 base 漂移都会在 Provider 访问前拒绝。每个 Continue 在内容提交之后更新 traversal；Complete 在一个 immediate transaction 中删除匹配 batch 的 run 行，并 upsert anchor 与 `completed_batch_id`。

### v0/v1/v2/v3 迁移、legacy bridge 与删除语义

旧 `provider_checkpoints` 只有 journal scope 且 JSON 严格等于 complete marker 的行可以证明“曾完整成功”，因此迁移为 `provider_sync_anchors` 的 NULL anchor。旧分页 cursor、listing/year scope、损坏或未知状态无法证明冻结窗口，全部丢弃；迁移随后删除旧表。v0/v1 还在同一事务中执行一次退役 Provider 名称重写。

v3 -> v4 在一个事务中新增 nullable `completed_batch_id` 和 `batch_id`，保留所有有效 anchor、run 和 lease。显式 `--file` 的默认 resume 可以把一个 mode 和 `started_at` 完全一致的 batchless checkpoint epoch 接入新 batch，并把同 epoch 的 completed anchors 一起标记；隐式全部 CSV 和混合 epoch 固定拒绝。这个 bridge 只为升级恢复，不把 Provider opaque 值解析或复制到 batch ledger。

切换 Provider 会自然使用新的 anchor/run namespace，而不修改内容库。删除或丢失全部 `data/index-control` 后，下一次运行从头抓取并通过 `article_identity_keys` 和 upsert 规则收敛。只删除 batch ledger 会失去 catalog completion/manifest recovery authority；仍带旧 batch ID 的 run checkpoint 会 fail closed，需恢复原 ledger、显式 `--no-resume`，或在 legacy 场景按 CLI 文档保留 catalog control 后重建单文件 batch。两类控制库都不需要恢复或备份；删除 catalog control 不是只清 cursor，而是同时放弃所有成功边界。

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

`users` 保存大小写不敏感的唯一用户名、密码 hash/salt、管理员标记、时间和单调 `token_generation`。新密码的 `password_hash` 是包含算法、版本、成本、salt 和输出的 `$argon2id$` PHC 字符串，旧 `salt` 列为空；旧 PBKDF2 hex 行保留原 salt，并在正确登录后通过原 hash+salt 的 CAS 单次升级。首个管理员只能通过 `litradar admin bootstrap` 创建；公开注册需要邀请码。密码变更、管理员重置与 `logout-all` 会在删除该用户全部 `access_tokens` 的同一 immediate transaction 中递增 `token_generation`；登录和依赖现有 token 的新令牌写入以该值为 CAS fence，后者还复核确切授权 token hash。目标用户不存在时密码更新返回未更新，删除令牌失败时密码与代际写入回滚。管理员标记更新和用户删除也使用 actor-aware immediate transaction：事务内重新验证 actor、target 和管理员计数，任何成功提交都至少保留一个 `is_admin = 1` 行。

`access_tokens` 保存唯一 token hash，不保存明文 token。`name='login'` 是浏览器 Cookie 会话的保留行；其他 active personal token 每用户最多 50 个。达到上限只阻止新建，不删除历史行。所有读取路径统一以 `expires_at <= now` 判定过期并清理；salt、原始 token 和邀请码由操作系统 CSPRNG 生成。

认证库 v15 为既有 `users` 行补入从 0 开始的 `token_generation`，不改写密码、用户 ID 或令牌。代际比较、授权 token 复核、新令牌插入和必需审计在同一个 `BEGIN IMMEDIATE` 中完成；任一条件已过期时整个签发无副作用回滚。

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

调度 repository 的新写入口接收 `SchedulerRunState`，不能持久化自由字符串；完成路径只接受终态。历史 `last_status = ''` 在读取时变为 `idle`，其他未识别旧值变为 `unknown`。delivery run/item/checkpoint/dedupe 已分别使用类型化状态与数据库 CHECK；worker 结果 JSON 和公开 manual/push statistics 也只序列化声明枚举。CNKI 状态来自上游，采用可保留原始字符串的 unknown 分支而不是拒绝或静默改写。

### 持久安全审计

认证库 v9 新增 `security_audit_events`。每行包含可空 `actor_id`/`target_id`、固定 `action`/`outcome`/`reason`、服务器 `request_id`、限流 `source_class`/`bucket`/计数以及 `occurred_at`。这些字段只保存分类和内部 ID，不保存用户名、密码、原始 token、邀请码、IP、Header 或请求内容。

索引覆盖 `(occurred_at, id)`、`(action, outcome, occurred_at)`、`(actor_id, occurred_at)` 和非空 `request_id`。触发器拒绝 `UPDATE`，从而保持追加语义；受控保留任务仍可 `DELETE`。安全变更 repository 在业务事务提交前插入必需审计行，插入失败会回滚业务变更。认证拒绝与限流通过独立 immediate transaction 同步追加。

`security_audit_maintenance` 只包含 `id=1` 和可空 `last_retention_at`。保留任务在同一 immediate transaction 中检查/更新该时间并删除最多 10,000 条过期记录，因此失败不会留下部分删除或错误推进窗口。默认保留 180 天，配置范围为 1–3650 天。

### `runtime_settings`

只接受[运行配置](configuration.md)列出的 20 个字段。四个字段的非空值以 `litradarenc:v1:` 密文保存：`openalex_api_key_pool`、`semantic_scholar_api_key_pool`、`cnki_captcha_token` 和 `provider_proxy_url`。它们都由同一秘密 registry 纳入迁移、验证和轮换；公开运行设置响应不会返回明文或持久密文。

`provider_proxy_policy`、Provider 路由/顺序和审计保留天数是非秘密运行配置。代理策略只保存逻辑 Provider 的布尔选择，代理 URL 只存在于认证库密文和当前进程受限内存，不进入内容库、索引控制库、worker request JSON、日志或审计 payload。

### `managed_meta_catalogs` 和 `announcements`

`managed_meta_catalogs` 记录官方 bundle 版本/hash 所有权，用于保护用户修改的目录。`announcements` 保存标题、消息、优先级、启用状态和时间。

## 数据库之外的状态

### 变更与投递状态

- `data/push_state/<db>.changes.json`：索引 update 的 Provider-neutral 变更清单；
- `data/push_state/<db>.json`：旧 notify/手动 PushPlus 状态导入源；
- `data/folder_push_state/<db>.json`：旧 push 状态导入源。

可变 checkpoint、run、item、dedupe 和 lease 的唯一权威存储是 `auth.sqlite`。运行时不再创建 `<db>.json` 或固定 `.tmp`，也不接受状态目录参数。清单的 `db_name` 是目标内容库身份；读取方不使用历史文件系统路径或 Provider 名称回退。旧导入源会原样保留，不能在导入后手工修改；`.changes.json` 始终保持文件契约，不进入认证库。

## 备份边界

v2 备份固定包含 `auth.sqlite` 和完整 `data/meta`，因此持久投递状态总在认证库快照中。`--include-indexes` 只包含 `data/index/*.sqlite` 内容库；`data/index-control` 永远排除，包括 `index-batches.sqlite` 和全部 catalog v4 controls，`data/index-work` 的 Crossref 工作集也始终排除。Provider-neutral `.changes.json` 和保留的旧导入源需要 `--include-push-state`。部署密钥始终单独保存。
