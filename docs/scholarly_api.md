# OpenAlex / Crossref / Unpaywall 集成说明

本文档说明英文期刊索引当前采用的 OpenAlex、Crossref、Unpaywall 接口方案。

验证日期：2026-05-05。

## 一、实现定位

推荐分工：

| 数据源 | 推荐角色 | 主要原因 |
| --- | --- | --- |
| Crossref | 文章列表主来源 | DOI 元数据、期刊维度检索、卷期页码、发布日期字段更稳定 |
| OpenAlex | 元数据增强来源 | 摘要、机构、ORCID、OA 状态、主题、PMID/PMCID、撤稿标记更丰富 |
| Unpaywall | OA 全文位置来源 | DOI 级别开放获取位置、PDF URL、landing page、license、OA 类型 |

不建议用 OpenAlex 单独作为主列表来源。测试中 OpenAlex 总量接近，但按 ISSN 与日期窗口拆到单刊后波动更明显；Crossref 更适合做期刊文章清单的主轴。

## 二、官方资料

| 来源 | 文档 |
| --- | --- |
| OpenAlex Works API | `https://developers.openalex.org/api-reference/works/list-works` |
| OpenAlex 分页 | `https://developers.openalex.org/guides/page-through-results` |
| OpenAlex 过滤语法 | `https://developers.openalex.org/guides/filtering` |
| Crossref REST API 技巧 | `https://www.crossref.org/documentation/retrieve-metadata/rest-api/tips-for-using-the-crossref-rest-api/` |
| Crossref REST API filters | `https://www.crossref.org/documentation/retrieve-metadata/rest-api/rest-api-filters/` |
| Unpaywall API | `https://unpaywall.org/products/api` |
| Unpaywall 字段说明 | `https://support.unpaywall.org/support/solutions/articles/44002142311-what-do-the-fields-in-the-api-response-and-snapshot-records-mean-` |
| Unpaywall best OA location | `https://support.unpaywall.org/support/solutions/articles/44001943223-how-is-the-best-oa-location-determined-` |

## 三、Crossref

基础地址：

- `https://api.crossref.org/v1`

核心端点：

| 端点 | 作用 |
| --- | --- |
| `GET /journals/{issn}/works` | 按 ISSN 获取期刊文章列表 |
| `GET /works/{doi}` | 按 DOI 获取单篇文章元数据 |
| `GET /works` | 全局 works 检索，可配合 `filter=issn:{issn}` 使用 |

建议请求参数：

| 参数 | 建议值 | 说明 |
| --- | --- | --- |
| `filter` | `type:journal-article,from-pub-date:YYYY-MM-DD` | 限定期刊论文与发布日期窗口 |
| `rows` | `1000` | Crossref 当前文档说明单页可提高到 1000 |
| `cursor` | `*` | 第一页使用 `*`，后续使用响应中的 `next-cursor` |
| `sort` | `published` 或 `indexed` | 首次全量可按发布日期，增量更适合按 indexed/update |
| `order` | `asc` 或 `desc` | 与 `sort` 配合 |
| `mailto` | 项目联系人邮箱 | 生产请求应带可联系邮箱 |

分页规则：

1. 第一页请求带 `cursor=*`。
2. 响应中读取 `message.next-cursor`。
3. 下一页继续传 `cursor={next-cursor}`。
4. 当本页 `items` 数量小于 `rows` 时停止。

建议保留字段：

| Crossref 字段 | 用途 |
| --- | --- |
| `DOI` | 主去重键，统一转为小写并去掉 URL 前缀 |
| `title` | 文章标题，通常是数组 |
| `author` | 作者列表 |
| `container-title` | 期刊名 |
| `ISSN` | 期刊 ISSN 列表 |
| `published-print` | 优先发布日期来源 |
| `published-online` | 次优先发布日期来源 |
| `published` / `issued` | 兜底发布日期来源 |
| `volume` | 卷 |
| `issue` | 期 |
| `page` | 页码 |
| `article-number` | 文章编号 |
| `abstract` | 摘要，可能含 JATS/XML 标记 |
| `license` | 出版方登记的许可信息 |
| `link` | 出版方登记的链接，不能等同于可直接下载全文 |
| `reference` | 参考文献列表，取决于出版方是否登记 |
| `relation` | 更正、撤稿、版本关系等 |
| `is-referenced-by-count` | Crossref 引用计数 |
| `indexed` | Crossref API 索引时间，适合增量同步 |

发布日期归一化顺序：

1. `published-print`
2. `published-online`
3. `published`
4. `issued`

增量同步不要只依赖 `from-pub-date`。发布日期可能被补录或修正，生产任务建议使用 `from-index-date` 或 `from-update-date` 做变更扫描。

## 四、OpenAlex

基础地址：

- `https://api.openalex.org`

核心端点：

| 端点 | 作用 |
| --- | --- |
| `GET /works` | 检索 works |
| `GET /works/{openalex_id}` | 按 OpenAlex ID 获取单篇 work |
| `GET /works/doi:{doi}` | 按 DOI 获取单篇 work |
| `GET /sources/issn:{issn}` | 按 ISSN 获取来源期刊信息 |

当前官方文档把 `api_key` 标为必填查询参数，生产环境应配置 OpenAlex API key。

建议请求参数：

| 参数 | 示例 | 说明 |
| --- | --- | --- |
| `filter` | `primary_location.source.issn:0957-1558,type:article` | 按主来源 ISSN 与类型过滤 |
| `filter` | `doi:https://doi.org/10.1287/mnsc.2020.0001` | 按 DOI 精确取数 |
| `cursor` | `*` | 深分页起始游标 |
| `per_page` | `100` | 当前官方文档的最大页大小 |
| `select` | `id,doi,title,publication_date,biblio,authorships,open_access,best_oa_location` | 减小响应体积 |
| `sort` | `publication_date` 或 `-publication_date` | 按发布日期排序 |

OpenAlex 支持在同一个 filter 内用 `|` 做 OR，当前文档示例说明一个 filter 最多组合 100 个值。按 DOI 批量增强时可以把多个 DOI 放在同一个 `doi:` filter 内。

建议保留字段：

| OpenAlex 字段 | 用途 |
| --- | --- |
| `id` | OpenAlex work ID |
| `doi` | DOI，通常是 `https://doi.org/...` 格式 |
| `title` / `display_name` | 标题 |
| `publication_year` | 出版年 |
| `publication_date` | 出版日期 |
| `type` | 文献类型 |
| `language` | 语言 |
| `cited_by_count` | OpenAlex 引用计数 |
| `is_retracted` | 撤稿标记 |
| `primary_location.source` | 主来源期刊、ISSN、ISSN-L、出版方 |
| `locations` | 其他位置 |
| `open_access` | OA 状态、OA URL、仓储全文标记 |
| `best_oa_location` | OpenAlex 判断的最佳 OA 位置 |
| `authorships` | 作者、ORCID、机构、国家、原始 affiliation |
| `ids` | DOI、PMID、PMCID 等外部 ID |
| `biblio` | 卷、期、起止页 |
| `abstract_inverted_index` | 倒排摘要，需要恢复为正文 |
| `referenced_works` | OpenAlex 引用对象 ID |
| `topics` / `primary_topic` | 学科主题 |
| `funders` / `awards` | 资助信息 |
| `has_content` / `content_url` | OpenAlex 内容服务标记，不等同于免费全文 |

`abstract_inverted_index` 不是直接文本。需要按词的位置索引恢复顺序，再拼成摘要。

## 五、Unpaywall

基础端点：

- `GET https://api.unpaywall.org/v2/{doi}?email={email}`

请求要求：

| 参数 | 说明 |
| --- | --- |
| `{doi}` | DOI，可以是裸 DOI |
| `email` | 必须是真实联系人邮箱；测试中假邮箱会被拒绝 |

Unpaywall 没有期刊 issue feed，也不会补全非 OA 文章全文。它只适合作为 DOI 级别 OA 位置解析器。

建议保留字段：

| Unpaywall 字段 | 用途 |
| --- | --- |
| `doi` | DOI |
| `is_oa` | 是否 OA |
| `oa_status` | gold、hybrid、green、bronze、closed 等状态 |
| `best_oa_location` | 最推荐的 OA 位置 |
| `oa_locations` | 所有 OA 位置 |
| `url_for_pdf` | PDF URL，存在时可尝试作为直接 PDF |
| `url_for_landing_page` | 落地页 URL |
| `license` | license |
| `version` | publishedVersion、acceptedVersion、submittedVersion 等 |
| `host_type` | publisher 或 repository |
| `repository_institution` | 仓储机构 |
| `journal_name` | 期刊名 |
| `journal_issns` | ISSN 字符串 |
| `published_date` / `year` | 出版日期或年份 |
| `updated` | Unpaywall 记录更新时间 |

## 六、推荐同步流程

1. 从项目 CSV 读取期刊名、ISSN、库 ID 等配置。
2. 用 Crossref `/journals/{issn}/works` 按期刊拉取文章列表。
3. 用 DOI 做主去重键；没有 DOI 的记录保留为低置信度记录。
4. 归一化 Crossref 的标题、作者、日期、卷、期、页码。
5. 对新增或变化 DOI 批量请求 OpenAlex，补摘要、机构、OA、PMID/PMCID、撤稿与主题字段。
6. 对新增或缓存过期 DOI 请求 Unpaywall，补 OA PDF 与 landing page。
7. 用 `(journal_id, publish_year, volume, issue)` 生成期次分组；缺少卷期的文章归入 in-press 或 unknown issue 桶。
8. 缓存原始响应与归一化结果，增量任务优先使用 Crossref `from-index-date` 或 `from-update-date`。

## 七、内容完整度观察

基于当前项目英文期刊集合的抽样对比，内容完整度预期如下：

| 指标 | 观察 |
| --- | --- |
| 文章数量 | Crossref 在测试时间窗口内覆盖更稳；OpenAlex 总量接近但单刊波动更大 |
| DOI 覆盖 | 当前英文路径 DOI 覆盖很高，Crossref/OpenAlex 适合用 DOI 串联 |
| 摘要覆盖 | OpenAlex 摘要覆盖通常优于 Crossref |
| OA 链接 | OpenAlex 与 Unpaywall 可补更多开放获取链接 |
| 机构与作者 ID | OpenAlex 可补 ORCID、机构与国家字段 |
| 卷期结构 | 当前方案需要从 Crossref 卷期字段合成 |
| 全文能力 | 当前方案只能稳定覆盖 OA 全文，不能替代机构授权解析 |

OpenAlex / Crossref / Unpaywall 不提供机构授权全文，只能提供 DOI 元数据与开放获取位置。`full_text_file` 字段在当前落库中保存 OA URL 或上游详情页，不再表示机构解析器地址。

## 八、风险点

| 风险 | 处理建议 |
| --- | --- |
| 出版方 Crossref 元数据缺失 | 允许 OpenAlex 补字段，但不要让 OpenAlex 覆盖更可靠的 Crossref 卷期页码 |
| OpenAlex ISSN 过滤漏刊 | 主列表用 Crossref；OpenAlex 只做 DOI 增强 |
| 日期字段含义不一致 | 统一以 Crossref 归一化发布日期为主，OpenAlex 日期为补充 |
| OA 链接失效 | Unpaywall/OpenAlex URL 做缓存但允许定期刷新 |
| 请求量过大 | 游标分页、按日期切片、缓存原始响应、遇到 4XX/429 退避 |
| 无 DOI 文章 | 无法使用 Unpaywall，OpenAlex 匹配也会显著变弱 |

## 九、落库建议

最低限度建议保留三类缓存：

| 缓存 | key | 用途 |
| --- | --- | --- |
| Crossref raw work | normalized DOI | 保留原始文章元数据与 indexed 时间 |
| OpenAlex raw work | normalized DOI 或 OpenAlex ID | 保留增强字段与更新时间 |
| Unpaywall raw record | normalized DOI | 保留 OA 位置与更新时间 |

最终展示字段应来自归一化层，而不是直接绑定任何单一接口字段。
