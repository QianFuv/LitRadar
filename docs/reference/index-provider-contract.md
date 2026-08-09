# 索引与 Provider 契约

本文档是 LitRadar 文章索引 Provider 的规范接入文档。当前契约版本为 `3`，实现来源是 `litradar-domain::index_contract`、`litradar-provider` 和 `litradar-index`；私有多进程 worker wire protocol 当前为 `6`。

核心边界只有三条：

1. LitRadar 维护期刊目录、规范化规则、稳定 ID、合并规则和数据库写入。
2. Provider 只接收规范目录项与同步 context，并返回规范期刊、期次、文章内容和 Provider 私有进度；它不能分配 ID、写数据库或返回持久链接。
3. 摘要页和全文是两个独立的可选在线能力，每次点击时解析，结果不写入索引库。

## 接入最小集合

一个只负责索引的 Provider 需要：

- 实现 `IndexContentProvider`；
- 把上游响应转换为 `ProviderBatch`；
- 在注册时只声明 `index_content=true`；
- 通过共享 conformance 测试。

在线能力均可省略。需要在线动作时，再分别实现：

- `ArticleAbstractProvider`：摘要页；
- `ArticleFullTextProvider`：全文跳转或有界文档。

索引能力不隐含任何在线能力，在线能力也不要求该 Provider 曾经索引这篇文章。

## LitRadar 维护的期刊目录

`data/meta/*.csv` 是 Provider 无关的规范目录。文件名 stem 是稳定内容库边界，例如 `chinese_journals.csv` 对应 `data/index/chinese_journals.sqlite`。

列顺序和含义：

| 列                           | 必填 | 规则                                                                                             |
| ---------------------------- | ---- | ------------------------------------------------------------------------------------------------ |
| `catalog_id`                 | 是   | 3–128 个小写 ASCII 字符；允许内部的 `.`、`_`、`-`；分配后不可因标题、ISSN 或 Provider 变化而重建 |
| `catalog_aliases`            | 否   | 以 `;` 分隔的已退役 catalog ID；不得等于当前 ID、相互重复或被其他规范期刊占用                    |
| `title`                      | 是   | 裁剪并规范化为 Unicode NFC 的规范标题                                                            |
| `issn`                       | 否   | 校验位正确的 `NNNN-NNNX` 印刷 ISSN                                                               |
| `eissn`                      | 否   | 校验位正确的电子 ISSN                                                                            |
| `all_issns`                  | 否   | 以 `;` 分隔的去重 ISSN；必须包含非空的 `issn`、`eissn`                                           |
| `title_aliases`              | 否   | 以 `;` 分隔；与规范标题及其他别名规范化后不得重复                                                |
| `area`                       | 否   | LitRadar 维护的领域标签                                                                          |
| `utd_rank`、`utd_rating`     | 否   | 维护的 UTD 排名信息                                                                              |
| `abs_rank`、`abs_rating`     | 否   | 维护的 ABS 排名信息                                                                              |
| `fms_rank`、`fms_rating`     | 否   | 维护的 FMS 排名信息                                                                              |
| `fmscn_rank`、`fmscn_rating` | 否   | 维护的 FMS China 排名信息                                                                        |

目录中禁止 `provider`、`source`、上游期刊 ID、路由、URL、可用性、Cookie、会话或检查点列。Provider 路由来自 `auth.sqlite.runtime_settings.index_provider_routes`，不属于目录内容。

## 规范内容类型

所有结构都拒绝未知序列化字段。

### `JournalCatalogEntry`

LitRadar 传给 Provider 的维护数据：当前 `catalog_id`、已退役 `catalog_aliases`、标题、ISSN 集、标题别名、领域和排名。Provider 只能读取，不能覆盖维护字段；Provider batch 仍只回显当前 `catalog_id`。

### `JournalDraft`

Provider 对所请求期刊的观察：

- 必须原样回显 `catalog_id`；
- 可提供 `observed_title`、`observed_issns` 和 `observed_title_aliases`；
- 观察标题必须匹配维护标题或别名；存在维护 ISSN 时，非空观察 ISSN 集至少共享一个值。

### `IssueDraft`

字段为 `catalog_id`、`publication_year`、`title`、`volume`、`number`、`date`。身份必须满足以下之一：

- 年份加卷或期号；
- 日期；
- 期次标题。

日期只接受 `YYYY`、`YYYY-MM` 或 `YYYY-MM-DD`；年份与日期同时存在时必须一致。

### `ArticleDraft`

| 分组 | 字段                                                                                          |
| ---- | --------------------------------------------------------------------------------------------- |
| 必填 | `catalog_id`、非空 `title`                                                                    |
| 出版 | `publication_year`、`date`、`issue_title`、`volume`、`issue_number`、`start_page`、`end_page` |
| 内容 | 有序 `authors[].display_name`、`abstract_text`                                                |
| 标识 | 规范 DOI、数字 PMID、按字典序排列且无重复的规范 `retraction_dois`                             |
| 状态 | 可空布尔值 `open_access`、`in_press`                                                          |

文章还必须具有 DOI、PMID，或同时具有出版时间和卷/期/起始页中的至少一个定位字段。禁止 Provider ID、持久 URL、原始响应、权限、订阅、馆藏、会话和传输状态。

### `ProviderBatch`

每次 `fetch` 返回一页：

- `catalog_id` 和 `journal.catalog_id` 必须回显请求值；
- `issues`、`articles` 必须全部属于该目录项；
- `ProviderProgress::Continue { checkpoint }` 必须携带非空且不超过 65,536 字节的 traversal checkpoint；
- `ProviderProgress::Complete { next_anchor }` 不再携带 traversal checkpoint，`next_anchor` 可为空；非空 anchor 同样最多 65,536 字节；
- Provider 不得假定 anchor 或 checkpoint 会永久存在。

### `IndexFetchContext` 与同步模式

核心每次调用 `IndexContentProvider::fetch(catalog, context)` 时传入：

- `mode`：`Bootstrap`、`Incremental` 或 `FullRescan`；
- `committed_anchor`：上一次整本期刊完整成功时提交的边界；
- `traversal_checkpoint`：本次冻结运行下一步要处理的位置。

三个 opaque 值（输入 anchor、输入 traversal、Complete 返回的 next anchor）都属于当前目录、Provider 和 `catalog_id` namespace。核心只检查为空、大小和进度形状，永远不解析 CNKI `year_issue_id`、Scholarly issue fingerprint、Crossref/OpenAlex cursor，也不把它们写入内容库。项目 batch ID 是核心恢复权限，不属于 `IndexFetchContext`，也不进入 worker request JSON。worker protocol v6 只携带相同 Provider context，并把当前 worker 所需的秘密单独通过 stdin bootstrap 传入；父进程独占 batch/control SQLite 和提交。因此完整 batch resume 不改变 Provider contract v3 或 worker protocol v6。

模式语义：

| 模式          | 核心语义                                                                                                      |
| ------------- | ------------------------------------------------------------------------------------------------------------- |
| `Bootstrap`   | 新 batch 完整覆盖；仅 active batch 内已经完成的 journal 可由默认 resume 零请求跳过                           |
| `Incremental` | 新 batch 从远端当前头部扫描到旧 committed anchor 并包含边界；同 active batch 已完成 journal 才是 skip marker |
| `FullRescan`  | 新 batch 覆盖完整 Provider 历史；可恢复同 batch/同模式 traversal，同 batch 已完成 journal 可跳过              |

journal 运行开始时核心把 committed anchor 冻结为 `base_anchor`。Provider 在自己的 traversal 中冻结 candidate head；重试不得根据已写内容重新计算边界。Continue 只推进 traversal。Complete 只有在整本期刊窗口已覆盖后才返回 next anchor。核心把 completion 与当前 batch ID 一起提交；成功 batch 结束后的下一条命令创建新 batch，因此 Provider 必须预期每次独立更新都会再次收到所有选中 journal。

提交顺序固定为：

```text
project batch admission + ordered catalog phase
-> same-batch completion check
-> Provider fetch + canonical content transaction
-> Continue: traversal checkpoint transaction
   Complete: delete matching batch run + replace committed anchor/batch marker in one transaction
-> --update only: persist exact manifest intent -> publish -> acknowledge -> optional notify
```

最终内容已提交而控制事务失败时，旧 anchor 保持不变；下次运行重放冻结窗口并由稳定 identity/upsert 收敛。

### 内置 Provider 的统一语义映射

- 国内 `cnki` 使用稳定期次树：从 candidate head 到 base issue 的闭区间，处理完 base 的全部 papers 页后 Complete。
- `scholarly` 使用规范字段生成 issue fingerprint，按出版日期降序，并用 anchor 日期过滤缩小 Crossref/OpenAlex 候选；过滤无法证明 base、candidate 可能回退或 OpenAlex 拒绝过滤时，在 Provider 内从无过滤头部重放。
- `cnki_oversea` 当前明确不支持增量边界；三种模式都完整扫描，成功返回 NULL anchor。核心不会为它解释海外期次句柄。

## 规范化与稳定身份

显示文本使用裁剪后的 Unicode NFC。用于比较的题名文本会转小写，把标点和空白折叠为空格。纯数字卷、期、页码会去除前导零。

- DOI：转小写，移除 `doi:` 或 `https://doi.org/` 前缀，只保存标识符。
- PMID：只允许数字并移除前导零。
- ISSN：统一为校验位正确的 `NNNN-NNNX`。

ID 由 `litradar-index` 独占生成：

- `journal_id` 来自不可变 `catalog_id` 和命名空间 `journal:v1`；
- 当前 catalog ID、全部 catalog alias 和全部 ISSN 通过 `journal_identity_keys` 归属于同一个规范 catalog ID；
- `issue_id` 来自 journal ID 加年份/卷/期；缺失时使用日期或标题 fallback；
- 文章依次建立 DOI、PMID、bibliographic fingerprint 三类 alias；新 ID 使用最强可用 alias，已有任一 alias 命中时复用原 ID。

bibliographic fingerprint 包含目录、规范题名、由 `publication_year` 或日期提取的年份、卷期和起始页。一个 draft 的 alias 只命中同一不可变文章时，不同 DOI 会作为该文章的多个 identity alias 保留；单值 `articles.doi` 使用规范 DOI 的字典序最小值，保证合并与重放不依赖到达顺序。PMID 仍禁止冲突；撤稿 DOI 以排序集合并集合并。

多个 alias 指向不同已有文章时明确报冲突，不猜测合并。系统不使用模糊题名、作者相似度、嵌入或在线查询做身份合并。

跨 Provider 保持 ID 的保证以共享规范 alias 为限。新 Provider 若无法提供任何与旧内容共享的 DOI、PMID 或 bibliographic fingerprint，系统会把它视为新文章；这不是兼容迁移机制。

## Provider 注册

`ProviderDescriptor` 包含：

- 2–64 字符的小写 ASCII 运行时名称；允许数字及非首位的 `_`、`-`；
- 三个显式 capability 布尔值：`index_content`、`article_abstract`、`article_full_text`；
- 只用于运行时响应校验的 `allowed_redirect_hosts`。

声明必须与实际提供的 trait object 完全一致；空能力、虚假声明、重复名称会拒绝注册。跳转域名必须是去重的小写规范主机名，且只能由声明了在线能力的 Provider 配置。域名列表不序列化到文章或数据库。

一个逻辑 Provider 可以在不同命令进程中分别注册实现。例如 `scholarly` 的索引实现只在 `index` 命令构造，摘要实现只在 `serve` 的 API 注册表构造；管理 API 再按同名 descriptor 聚合 capability。这里的“分进程注册”不增加常驻服务，也不表示不同 Provider 之间自动回退。

## Provider 托管代理边界

`provider_proxy_url` 提供一个加密的全局 HTTP、HTTPS、SOCKS5 或 SOCKS5h authority，`provider_proxy_policy` 按逻辑 Provider 名称独立选择是否使用。它们是运行配置，不是 Provider descriptor、目录、`IndexFetchContext`、batch、anchor 或 checkpoint 的一部分。完整 URL 语法、DNS 差异、启用/清除步骤和流量归属见[运行配置](configuration.md)。

每个 Provider client 都从一个显式决定构造：

- 关闭开关时使用受管直连，并明确禁用 `HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY` 等环境发现。
- 打开开关时，所有匹配请求只使用配置的显式代理；代理不可达或请求失败会按现有有界错误/重试规则失败，不会改成直连。
- URL 和 policy 在进程启动时一起验证；任一启用项没有 URL、未知 Provider 或无效 URL 都会在 Provider 请求前失败。

直接索引模式在父进程内按所选 `provider_name` 取得决定，并只通过内存交给该注册。多进程模式则遵守 protocol v6 的秘密边界：

1. 父进程按 worker request 中的 `provider_name` 选择代理；未启用时选择为空。
2. 可丢弃 worker request JSON、进程参数和 child 环境都不含代理 URL。
3. 父进程启动 child 后，通过 stdin 发送一次带 protocol version、worker ID 和可选 `provider_proxy_url` 的 bootstrap；URL 只会发给自身逻辑 Provider 已启用的 worker。
4. child 在构造 Provider 前验证 protocol version 与 worker ID，并消费该值；后续同一 stdin 流只传 durable commit ACK。
5. bootstrap、Provider proxy selection、错误和 Debug 只暴露 direct/explicit 或固定脱敏状态，不暴露 authority、userinfo 或完整 URL。

该 stdin 字段是内部秘密传输，不是公共配置、可日志化诊断字段或向第三方 Provider 开放的扩展点。AI、通知/PushPlus、MCP 和 API 返回给浏览器的 HTTPS redirect 不进入这条代理链路。

## 在线文章能力

API 从内容库构造 `ArticleLocator`，其中只有规范文章元数据和内部 ID。Provider 得不到索引来源信息或存储链接。

运行设置分别给出 default 和 per-catalog 有序列表：

- `article_abstract_provider_orders`；
- `article_fulltext_provider_orders`。

每个配置使用 `{ "default": [...], "catalogs": { "<stem>": [...] } }`。catalog 条目完整替换 default，缺少条目表示继承，显式空数组只禁用该 CSV/同名内容库的动作。解析器忽略未注册或未声明相应能力的名称，并依次尝试其余 Provider。超时、未找到、暂时不可用、无效结果和需要认证都允许后续 Provider fallback；全部失败后才返回稳定的不可用或认证错误。索引 Provider 路由与这两条在线链路相互独立。

前端“文章详情”是读取已存规范字段的本地弹窗，不是 Provider trait 或在线路由。公共在线动作只保留摘要页和全文。

### 结果契约

摘要页返回临时 `ArticleRedirect`。全文返回：

- 临时 HTTPS redirect；或
- `ArticleFullTextDocument`，包含安全 MIME、可选安全文件名和最多 32 MiB 的非空字节。

所有 redirect 必须：

- 长度不超过 8,192 字符；
- 使用 HTTPS；
- 没有 user-info、控制字符或空 authority；
- 精确匹配该 Provider 注册的允许域名。

API 用 `307 Temporary Redirect` 或文档响应返回结果，并设置 `Cache-Control: private, no-store`。结果、URL、下载文件和访问时间不写入内容库、控制库、认证库或文件缓存。Provider 可读取当前用户已有的认证会话，但一次文章动作不能创建、更新或 touch 会话。

## 内容库与控制库

| 路径                                         | 生命周期 | 内容                                                                                   |
| -------------------------------------------- | -------- | -------------------------------------------------------------------------------------- |
| `data/index/<catalog>.sqlite`                | 需要备份 | v6 规范期刊、期刊/文章 identity aliases、撤稿关系、列表投影、FTS 和文章变更 outbox     |
| `data/index-control/index-batches.sqlite`    | 可丢弃   | v2 core-owned batch fingerprint、catalog phase/manifest intent、typed notify handoff 和全局 lease |
| `data/index-control/<catalog>.sqlite`        | 可丢弃   | v4 Provider-scoped lease、batch-aware 成功 anchor 和运行 traversal checkpoint            |

成功 anchor 与运行 checkpoint 分表保存，并分别带可空 `completed_batch_id` / `batch_id`。成功行存在但 anchor 为 NULL 表示“完整成功但没有可复用边界”，不同于成功行缺失；它只有在完成标记属于 active batch 时才能跳过，否则新 batch 安全完整覆盖。删除控制状态会失去 batch、成功边界和恢复进度，并依靠 alias/upsert 收敛，不会改变内容身份。

每次目录运行在构造 Provider、分配 worker 或发出请求之前完成期刊身份预检。当前目录的 catalog ID、退役 catalog alias 和全部 ISSN 必须唯一归属于同一个规范 catalog ID；已有规范 journal 的标题、别名、ISSN、领域、排名及 listing/FTS 投影会在同一内容事务中收敛。即使当前 catalog ID 已有成功 anchor 行，这一步仍会执行，随后 Bootstrap 才可能以零 Provider 请求跳过该期刊。空内容库只登记身份键，不创建 journal 壳。

旧 catalog alias 若在任意 Provider namespace 下仍有 anchor 或 run checkpoint，运行固定失败；系统不会把 opaque 状态搬到当前 catalog ID。旧 alias journal 只有在不存在 issue、article、listing 和 outbox 历史时才可由事务清理。非空旧实体、身份所有权冲突和确定性 ID 冲突都在 Provider 请求前原子失败；内容 batch 写入时还会复核所有权。

内容库禁止 Provider 名称、路由、检查点、lease、运行统计、上游 ID 和 URL。控制库禁止规范文章内容；batch ledger 不保存 Provider opaque state、代理或凭据。备份明确排除整个 `data/index-control`。

## Conformance 流程

新增 Provider 至少应执行：

1. 用规范 `JournalCatalogEntry` fixture 调用每个声明能力。
2. 对索引结果运行 `validate_index_provider_fixture`。
3. 对在线能力分别运行 `validate_abstract_provider_fixture`、`validate_full_text_provider_fixture`。
4. 覆盖上游字段变体，证明它们产生相同的规范 `ArticleDraft`。
5. 覆盖错误分类、分页结束、重复 traversal checkpoint、无效重定向、超大文档和秘密脱敏。
6. 运行 Provider 注册矩阵，证明未实现能力不被声明。
7. 运行 Provider switch fixture，证明共享 alias 复用同一 ID，且新 Provider 使用独立 anchor/run namespace。
8. 对发出 HTTP 的实现覆盖受管直连、显式代理失败不直连回退，以及多进程 request/参数/环境/日志不含代理秘密。

内置实现的常用检查：

```bash
cargo test -p litradar-domain -p litradar-provider -p litradar-sources -p litradar-index
cargo clippy -p litradar-provider -p litradar-sources -p litradar-index --all-targets --all-features -- -D warnings
```

## 更换索引 Provider

以 `chinese_journals` 为例：

1. 保持 CSV 文件名和每行 `catalog_id` 不变。
2. 新 Provider 映射到相同规范类型并通过 conformance 测试。
3. 注册 Provider，只声明实际能力。
4. 把 `index_provider_routes` 中的 `chinese_journals` 改为新运行时名称。
5. 备份内容库；控制库无需迁移。
6. 运行索引并检查共享 alias 的 ID/count 对比。

不需要替换 v6 内容库；精确 v4/v5 内容库会原子迁移到 v6。不要把旧 Provider anchor 或 traversal checkpoint 复制给新 Provider；两个 namespace 可同时存在于可丢弃控制库，新 Provider 首次运行安全完整覆盖。摘要页和全文 Provider 顺序独立配置，不必跟随索引 Provider 一起切换。

## v6 升级与旧版本重建

应用只接受：

- 不存在的新文件；
- 完全空的 v0 SQLite；
- schema 精确匹配、可在一个事务内迁移的 v4 或 v5 内容库；
- schema 精确匹配的 v6 内容库。

v4 会先增加 `journal_identity_keys` 及其索引，再与 v5 一样迁移到 v6。v5 到 v6 把撤稿关系规范化为 `article_retraction_dois`，不携带旧单值 `articles.retraction_doi`；除此之外不重映射内容 ID，也不改变 projection 或 outbox。v0 非空库及 v1–v3 索引库不会迁移到 v6。

只有需要重建 v1–v3 时，才在执行任何移动或删除前确认以下影响：旧内容不会导入 v6；重建会使用新的规范身份空间；旧 favorite/tracking 中的 article ID 可能变成陈旧引用。应用不会自动删除、重命名或改写旧库，也不会迁移或清理这些引用。

遇到 rebuild-required 错误时使用以下顺序：

1. 停止 `litradar serve` 和所有独立索引写入进程。
2. 创建并验证包含旧内容库的备份。
3. 记录错误中给出的确切文件路径和可用于重建后比较的期刊/文章数量。
4. 优先把该确切旧索引文件移动到备份位置；确认不再需要回退时才删除。不要使用目录级通配删除。
5. 从未改名的维护目录重新运行索引。
6. 验证 v6 schema、目录期刊数、文章数和抽样内容，再恢复服务。
7. 明确决定保留、导出或清理无法解析的旧 favorite/tracking 引用；LitRadar 不会代替运维人员作此决定。
