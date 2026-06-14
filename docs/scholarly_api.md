# Crossref / OpenAlex / Semantic Scholar 集成说明

本文档说明英文期刊索引当前采用的 Crossref、OpenAlex、Semantic Scholar 接口方案。

验证日期：2026-06-14。

## 一、实现定位

当前英文路径按 DOI 串联三个来源：

| 数据源 | 当前角色 | 主要原因 |
| --- | --- | --- |
| Crossref | 文章列表主来源 | DOI 元数据、期刊维度检索、卷期页码、发布日期字段更适合按 ISSN 构建期刊清单 |
| OpenAlex | 元数据增强来源 | 摘要、作者机构、PMID/PMCID、OA 状态和 OA fallback URL 更丰富 |
| Semantic Scholar | OA/PDF 批量增强来源 | `/graph/v1/paper/batch` 可按 DOI 批量获取 `isOpenAccess` 和 `openAccessPdf` |

不建议用 OpenAlex 或 Semantic Scholar 单独替代 Crossref 作为期刊文章列表来源。Crossref 的 `/journals/{issn}/works` 更贴合当前 CSV 期刊索引模型；OpenAlex 和 Semantic Scholar 只按 DOI 补充字段。

## 二、官方资料

| 来源 | 文档 |
| --- | --- |
| Crossref REST API 技巧 | `https://www.crossref.org/documentation/retrieve-metadata/rest-api/tips-for-using-the-crossref-rest-api/` |
| Crossref REST API filters | `https://www.crossref.org/documentation/retrieve-metadata/rest-api/rest-api-filters/` |
| OpenAlex Works API | `https://developers.openalex.org/api-reference/works/list-works` |
| OpenAlex 分页 | `https://developers.openalex.org/guides/page-through-results` |
| OpenAlex 过滤语法 | `https://developers.openalex.org/guides/filtering` |
| Semantic Scholar API | `https://www.semanticscholar.org/product/api` |
| Semantic Scholar Graph OpenAPI | `https://api.semanticscholar.org/graph/v1/swagger.json` |

## 三、Crossref

基础地址：

- `https://api.crossref.org/v1`

当前使用端点：

| 端点 | 作用 |
| --- | --- |
| `GET /journals/{issn}/works` | 按 ISSN 获取期刊文章列表 |

当前请求参数：

| 参数 | 值 | 说明 |
| --- | --- | --- |
| `filter` | `type:journal-article`，增量时附加 `from-pub-date:YYYY-MM-DD` | 限定期刊论文与发布日期窗口 |
| `rows` | `1000` | 单页最大记录数 |
| `cursor` | `*`，后续使用 `message.next-cursor` | 游标分页 |
| `sort` | `published` | 按发布日期排序 |
| `order` | `asc` | 从旧到新处理 |
| `mailto` | 来自 `CROSSREF_MAILTO_POOL` | 生产请求应带可联系邮箱 |

当前落库使用字段：

| Crossref 字段 | 用途 |
| --- | --- |
| `DOI` | 主去重键，统一转为小写并去掉 URL 前缀 |
| `title` | 文章标题 |
| `author` | 作者列表 |
| `published-print` / `published-online` / `published` / `issued` | 发布日期归一化来源 |
| `volume` / `issue` / `page` / `article-number` | 期次与页码 |
| `abstract` | 摘要，可能含 JATS/XML 标记 |
| `relation` | 更正、撤稿、版本关系等 |
| `URL` | `content_location` 的 Crossref fallback |

## 四、OpenAlex

基础地址：

- `https://api.openalex.org`

当前使用端点：

| 端点 | 作用 |
| --- | --- |
| `GET /works` | 用 DOI filter 批量补充 work 元数据 |

当前请求参数：

| 参数 | 值 | 说明 |
| --- | --- | --- |
| `filter` | `doi:https://doi.org/{doi1}|https://doi.org/{doi2}` | 当前每批最多 100 个 DOI |
| `per-page` | 当前 batch 大小 | 与 DOI filter 数量保持一致 |
| `select` | 只请求当前落库需要的字段 | 降低响应体积 |
| `api_key` | 来自 `OPENALEX_API_KEY_POOL` | 配置后随请求发送 |
| `mailto` | 来自 `CROSSREF_MAILTO_POOL` | 复用联系人邮箱池 |

当前落库使用字段：

| OpenAlex 字段 | 用途 |
| --- | --- |
| `title` | Crossref 标题缺失时补充 |
| `publication_date` | Crossref 日期缺失时补充 |
| `abstract_inverted_index` | 恢复为摘要正文 |
| `authorships` | Crossref 作者缺失时补充 |
| `ids.pmid` | PMID |
| `open_access.is_oa` | `open_access` fallback |
| `best_oa_location.pdf_url` | `full_text_file` fallback |
| `best_oa_location.landing_page_url` | `content_location` 优先来源 |

## 五、Semantic Scholar

基础地址：

- `https://api.semanticscholar.org/graph/v1`

当前使用端点：

| 端点 | 作用 |
| --- | --- |
| `POST /paper/batch` | 按 DOI 批量获取论文 OA 信息 |

当前请求：

```text
POST /graph/v1/paper/batch?fields=externalIds,url,isOpenAccess,openAccessPdf
Header: x-api-key: <SEMANTIC_SCHOLAR_API_KEY_POOL selected key>
Body: {"ids": ["DOI:10.0000/example", "..."]}
```

关键约束：

- `SEMANTIC_SCHOLAR_API_KEY_POOL` 为空时跳过 Semantic Scholar enrichment。
- 单次 batch 最多 500 个 paper IDs。
- 当前按官方 introductory limit 保守处理为全局 1 RPS。
- 多进程索引时，Semantic Scholar 请求使用 source-aware throttle 做进程间错峰，避免每个进程各自打满 1 RPS。
- `url` 是 Semantic Scholar 页面地址，不写入 `content_location`。

当前落库使用字段：

| Semantic Scholar 字段 | 用途 |
| --- | --- |
| `externalIds.DOI` | 响应归并到 normalized DOI |
| `isOpenAccess` | 参与 `open_access` 计算 |
| `openAccessPdf.url` | 优先写入 `full_text_file` |

## 六、并发与限速

`--processes` 控制期刊级并行；`--workers` 仍用于 CNKI 详情抓取等高并发路径。英文 scholarly 路径现在在 `ScholarlyClient` 内使用 source-aware throttle：

| Source | 进程内并发 | 请求间隔 | 说明 |
| --- | --- | --- | --- |
| Crossref | 1 | 0 秒 | 保持原有串行分页语义 |
| OpenAlex | 1 | 0 秒 | DOI batch 串行处理 |
| Semantic Scholar | 1 | 1 秒全局错峰 | 按 process count 放大单进程间隔，整体接近 1 RPS |

提高吞吐时优先使用 batch，而不是增加 Semantic Scholar 并发。

## 七、当前同步流程

1. 从项目 CSV 读取期刊名、ISSN、库 ID 等配置。
2. 用 Crossref `/journals/{issn}/works` 按期刊拉取文章列表。
3. 用 DOI 做主去重键；没有 DOI 的记录仍按 Crossref URL 生成低置信度记录。
4. 用 Crossref 字段归一化标题、作者、日期、卷、期、页码。
5. 对本轮需要处理的 DOI 批量请求 OpenAlex，补摘要、作者、PMID、OA fallback 与 landing page。
6. 对本轮需要处理的 DOI 批量请求 Semantic Scholar，补 OA flag 与 PDF URL。
7. 用 `(journal_id, publish_year, volume, issue)` 生成期次分组；缺少卷期的文章归入 in-press。
8. 写入 `journals`、`issues`、`articles`、`article_listing` 和 `article_search`。

## 八、落库字段语义

英文路径当前关键字段来源：

| 字段 | 来源顺序 |
| --- | --- |
| `open_access` | Semantic Scholar `isOpenAccess`，然后 OpenAlex `open_access.is_oa` |
| `full_text_file` | Semantic Scholar `openAccessPdf.url`，然后 OpenAlex `best_oa_location.pdf_url` / `landing_page_url` |
| `content_location` | OpenAlex `best_oa_location.landing_page_url`，然后 Crossref `URL`，最后 DOI URL |
| `permalink` | DOI URL；无 DOI 时使用 `content_location` |

`full_text_file` 只表示上游可用的 OA PDF 或详情 URL，不代表机构授权全文。`content_location` 表示更适合展示给用户的落地页，不使用 Semantic Scholar 页面地址。

## 九、风险点

| 风险 | 处理建议 |
| --- | --- |
| 出版方 Crossref 元数据缺失 | 允许 OpenAlex 补字段，但不要让 OpenAlex 覆盖更可靠的 Crossref 卷期页码 |
| OpenAlex DOI enrichment 缺失 | 保留 Crossref 文章记录，只缺少增强字段 |
| Semantic Scholar 无 key 或无 DOI 结果 | 跳过 S2 enrichment，使用 OpenAlex/Crossref fallback |
| Semantic Scholar 429 或 5xx | 使用 source throttle 和 retry/backoff；不要通过并发硬冲 |
| OA 链接失效 | 允许后续增量任务刷新；展示层应能处理失效链接 |
| 无 DOI 文章 | 不能使用 DOI batch enrichment，只保留 Crossref URL fallback |
