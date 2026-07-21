# 索引与 Provider 契约

本文档是 LitRadar 文章索引 Provider 的规范接入文档。当前契约版本为 `2`，实现来源是 `litradar-domain::index_contract`、`litradar-provider` 和 `litradar-index`。

核心边界只有三条：

1. LitRadar 维护期刊目录、规范化规则、稳定 ID、合并规则和数据库写入。
2. Provider 只接收规范目录项并返回规范期刊、期次和文章内容；它不能分配 ID、写数据库或返回持久链接。
3. 详情页、摘要页和全文是三个独立的可选在线能力，每次点击时解析，结果不写入索引库。

## 接入最小集合

一个只负责索引的 Provider 需要：

- 实现 `IndexContentProvider`；
- 把上游响应转换为 `ProviderBatch`；
- 在注册时只声明 `index_content=true`；
- 通过共享 conformance 测试。

在线能力均可省略。需要在线动作时，再分别实现：

- `ArticleDetailProvider`：详情页；
- `ArticleAbstractProvider`：摘要页；
- `ArticleFullTextProvider`：全文跳转或有界文档。

索引能力不隐含任何在线能力，在线能力也不要求该 Provider 曾经索引这篇文章。

## LitRadar 维护的期刊目录

`data/meta/*.csv` 是 Provider 无关的规范目录。文件名 stem 是稳定内容库边界，例如 `chinese_journals.csv` 对应 `data/index/chinese_journals.sqlite`。

列顺序和含义：

| 列                           | 必填 | 规则                                                                                             |
| ---------------------------- | ---- | ------------------------------------------------------------------------------------------------ |
| `catalog_id`                 | 是   | 3–128 个小写 ASCII 字符；允许内部的 `.`、`_`、`-`；分配后不可因标题、ISSN 或 Provider 变化而重建 |
| `catalog_aliases`            | 否   | 以 `;` 分隔的已退役 catalog ID；不得等于当前 ID、相互重复或被其他规范期刊占用                   |
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
| 标识 | 规范 DOI、数字 PMID、按字典序排列且无重复的规范 `retraction_dois`                            |
| 状态 | 可空布尔值 `open_access`、`in_press`                                                          |

文章还必须具有 DOI、PMID，或同时具有出版时间和卷/期/起始页中的至少一个定位字段。禁止 Provider ID、持久 URL、原始响应、权限、订阅、馆藏、会话和传输状态。

### `ProviderBatch`

每次 `fetch` 返回一页：

- `catalog_id` 和 `journal.catalog_id` 必须回显请求值；
- `issues`、`articles` 必须全部属于该目录项；
- `is_complete=true` 时 `next_checkpoint` 必须为空；
- `is_complete=false` 时必须返回非空且不超过 65,536 字节的 opaque checkpoint；
- Provider 不得假定 checkpoint 会永久存在。

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
- 四个显式 capability 布尔值；
- 只用于运行时响应校验的 `allowed_redirect_hosts`。

声明必须与实际提供的 trait object 完全一致；空能力、虚假声明、重复名称会拒绝注册。跳转域名必须是去重的小写规范主机名，且只能由声明了在线能力的 Provider 配置。域名列表不序列化到文章或数据库。

## 在线文章能力

API 从内容库构造 `ArticleLocator`，其中只有规范文章元数据和内部 ID。Provider 得不到索引来源信息或存储链接。

运行设置分别给出有序列表：

- `article_detail_provider_order`；
- `article_abstract_provider_order`；
- `article_fulltext_provider_order`。

解析器忽略未注册或未声明相应能力的名称，并依次尝试其余 Provider。超时、未找到、暂时不可用、无效结果和需要认证都允许后续 Provider 回退；全部失败后才返回稳定的不可用或认证错误。

### 结果契约

详情和摘要页返回临时 `ArticleRedirect`。全文返回：

- 临时 HTTPS redirect；或
- `ArticleFullTextDocument`，包含安全 MIME、可选安全文件名和最多 32 MiB 的非空字节。

所有 redirect 必须：

- 长度不超过 8,192 字符；
- 使用 HTTPS；
- 没有 user-info、控制字符或空 authority；
- 精确匹配该 Provider 注册的允许域名。

API 用 `307 Temporary Redirect` 或文档响应返回结果，并设置 `Cache-Control: private, no-store`。结果、URL、下载文件和访问时间不写入内容库、控制库、认证库或文件缓存。Provider 可读取当前用户已有的认证会话，但一次文章动作不能创建、更新或 touch 会话。

## 内容库与控制库

| 路径                                  | 生命周期 | 内容                                                                       |
| ------------------------------------- | -------- | -------------------------------------------------------------------------- |
| `data/index/<catalog>.sqlite`         | 需要备份 | v6 规范期刊、期刊/文章 identity aliases、撤稿关系、列表投影、FTS 和文章变更 outbox |
| `data/index-control/<catalog>.sqlite` | 可丢弃   | v1 Provider-scoped lease 和 opaque checkpoint                              |

内容提交先完成，检查点随后提交。若检查点提交失败，重跑会重新读取已写内容并依靠 alias/upsert 收敛；不会因控制状态丢失而复制文章。删除控制库只会失去恢复进度，不会改变内容身份。

每次目录运行在构造 Provider、分配 worker 或发出请求之前完成期刊身份预检。当前目录的 catalog ID、退役 catalog alias 和全部 ISSN 必须唯一归属于同一个规范 catalog ID；已有规范 journal 的标题、别名、ISSN、领域、排名及 listing/FTS 投影会在同一内容事务中收敛。即使当前 catalog ID 的 checkpoint 已是 `complete`，这一步仍会执行，随后才以零 Provider 请求恢复该期刊。空内容库只登记身份键，不创建 journal 壳。

旧 catalog alias 若在任意 Provider namespace 下仍有 journal 或 year checkpoint，运行固定失败；系统不会把 opaque checkpoint 搬到当前 catalog ID。旧 alias journal 只有在不存在 issue、article、listing 和 outbox 历史时才可由事务清理。非空旧实体、身份所有权冲突和确定性 ID 冲突都在 Provider 请求前原子失败；内容 batch 写入时还会复核所有权。

内容库禁止 Provider 名称、路由、检查点、lease、运行统计、上游 ID 和 URL。控制库禁止规范文章内容。备份明确排除 `data/index-control`。

## Conformance 流程

新增 Provider 至少应执行：

1. 用规范 `JournalCatalogEntry` fixture 调用每个声明能力。
2. 对索引结果运行 `validate_index_provider_fixture`。
3. 对在线能力分别运行 `validate_detail_provider_fixture`、`validate_abstract_provider_fixture`、`validate_full_text_provider_fixture`。
4. 覆盖上游字段变体，证明它们产生相同的规范 `ArticleDraft`。
5. 覆盖错误分类、分页结束、重复 checkpoint、无效重定向、超大文档和秘密脱敏。
6. 运行 Provider 注册矩阵，证明未实现能力不被声明。
7. 运行 Provider switch fixture，证明共享 alias 复用同一 ID，且新 Provider 使用独立控制 checkpoint。

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

不需要替换 v6 内容库；精确 v4/v5 内容库会原子迁移到 v6。不要把旧 Provider checkpoint 复制给新 Provider；两个 namespace 可同时存在于可丢弃控制库。详情、摘要和全文 Provider 顺序独立配置，不必跟随索引 Provider 一起切换。

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
