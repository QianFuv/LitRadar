# Scholarly 数据源

本文档说明 `source=scholarly` 的实际索引链路。它同时记录当前代码行为与上游公开约束；两者不一致时会明确标注，而不是把期望行为写成既成事实。

上游约束核对日期：2026-07-16。

## 职责分工

| 数据源           | 当前职责                                                                                             |
| ---------------- | ---------------------------------------------------------------------------------------------------- |
| Crossref         | 按期刊 ISSN 获取文章清单，并提供 DOI、卷期页码、日期与基础元数据                                     |
| OpenAlex         | 补充摘要、作者、PMID、OA 状态和落地页；Crossref 对全部 ISSN 返回 404 时还承担期刊与文章清单 fallback |
| Semantic Scholar | 按 DOI 批量补充摘要、OA 状态和 PDF URL                                                               |

索引开始前，只要输入 CSV 含有 `scholarly` 行，就必须同时配置 `openalex_api_key_pool` 与 `semantic_scholar_api_key_pool`。`crossref_mailto_pool` 可选，但生产环境应配置可联系邮箱。

配置入口与秘密字段更新方式见[配置参考](../configuration.md)。

## 同步流程

对每一行期刊配置，索引器执行：

1. 从 `issn`、`all_issns` 等字段生成有序 ISSN 候选。
2. 依次请求 Crossref `/journals/{issn}/works`。
3. 第一个非 404 响应成为主文章清单；其他错误直接终止该期刊。
4. 只有全部 ISSN 候选都返回 404 时，才先按 ISSN、再按题名解析 OpenAlex source，并按 source 拉取 works。
5. 按规范化 DOI 请求增强：OpenAlex DOI 查询保持每批最多 100 条，Semantic Scholar 对当前来源页统一去重后请求。
6. 合并字段，生成期刊、期次、文章、列表辅助表和 FTS 数据。
7. 将实际解析来源写入 `journal_meta.resolved_source` 及对应的 resolved 字段。

Crossref 返回成功但清单为空时，不会触发 OpenAlex source fallback。没有 DOI 的 Crossref 记录可以落库，但不能进入 DOI 批量增强。

## Crossref

基础地址：`https://api.crossref.org/v1`

当前请求：

| 项目     | 实现                                               |
| -------- | -------------------------------------------------- |
| 端点     | `GET /journals/{issn}/works`                       |
| 过滤     | 始终使用 `type:journal-article`；可信增量窗口再追加 `from-update-date` |
| 分页     | 从 `cursor=*` 开始，随后使用 `message.next-cursor` |
| 页大小   | `rows=225`                                         |
| 顺序     | `sort=published&order=asc`                         |
| 联系信息 | 使用 `crossref_mailto_pool` 中第一个非空值         |

当一页少于 225 条或响应不再提供下一游标时停止。225 是索引器为有界内存和页级增强选择的实现值；官方文档确认 `rows` 最大值为 1000，并推荐使用 cursor 深分页：

- [REST API 使用建议](https://www.crossref.org/documentation/retrieve-metadata/rest-api/tips-for-using-the-crossref-rest-api/)
- [REST API filters](https://www.crossref.org/documentation/retrieve-metadata/rest-api/rest-api-filters/)

已有可信期刊完成时间时，live update 从该时间向前重叠 30 天，并将同一个 `from-update-date` 传给该期刊的每个 Crossref 游标页。全量索引或不可信检查点不添加日期过滤。

主要字段：

| 目标字段        | 来源                                                                         |
| --------------- | ---------------------------------------------------------------------------- |
| DOI / 平台标识  | `DOI`；无 DOI 时回退到 `URL`                                                 |
| 标题、作者      | `title`、`author`                                                            |
| 日期            | `published-print`、`published-online`、`published`、`issued` 的有序 fallback |
| 卷期页码        | `volume`、`issue`、`page`、`article-number`                                  |
| 摘要            | `abstract`，落库前移除标记                                                   |
| 撤稿关系        | `relation`                                                                   |
| 落地页 fallback | `URL`                                                                        |

## OpenAlex

基础地址：`https://api.openalex.org`

当前用途：

| 端点           | 查询方式                                          | 用途                              |
| -------------- | ------------------------------------------------- | --------------------------------- |
| `GET /sources` | `filter=issn:{issn}`，每页 5 条                   | 解析 Crossref 404 的期刊          |
| `GET /sources` | `search={title}`，每页 5 条                       | ISSN 无结果后的题名 fallback      |
| `GET /works`   | `doi:url1\|url2...`                               | 每批最多 100 个 DOI 的增强        |
| `GET /works`   | `primary_location.source.id:{source_id}` + cursor | OpenAlex source 文章清单 fallback |

所有请求会发送配置池中的第一个 OpenAlex key；若配置了 Crossref mailto，也会一并发送。请求通过 `select` 限制到落库所需字段。

主要增强字段：

| 目标字段       | 来源                                |
| -------------- | ----------------------------------- |
| 标题、日期     | `title`、`publication_date`         |
| 摘要           | `abstract_inverted_index` 还原      |
| 作者           | `authorships`                       |
| PMID           | `ids.pmid`                          |
| OA 状态        | `open_access.is_oa`                 |
| PDF / 全文候选 | `best_oa_location.pdf_url`          |
| 落地页         | `best_oa_location.landing_page_url` |

官方当前契约要求 API key，`per_page` 范围为 1–100；OR 过滤适合把最多 100 个 ID 合并到一次查询中：

- [List works](https://developers.openalex.org/api-reference/works/list-works)
- [List sources](https://developers.openalex.org/api-reference/sources/list-sources)
- [API recipes](https://developers.openalex.org/guides/recipes)

### 已知兼容性风险

当前 `source_works` fallback 把 `per-page` 设置为 200，而 OpenAlex 当前官方上限是 100。这是实现与上游契约的偏差：只有“Crossref 对全部 ISSN 返回 404，且转入 OpenAlex source works”时才会触发。本文保留事实说明，不把 200 描述为受支持值；修复应在代码任务中把页大小和终止条件一起调整并增加回归测试。

## Semantic Scholar

基础地址：`https://api.semanticscholar.org/graph/v1`

当前请求：

```http
POST /paper/batch?fields=externalIds,url,isOpenAccess,openAccessPdf,abstract
x-api-key: <configured key>
Content-Type: application/json

{"ids":["DOI:10.0000/example"]}
```

索引器先对当前来源页的 DOI 统一规范化和去重：Crossref 页最多 225 条，OpenAlex fallback 页当前最多 200 条，因此通常只占用一次 Semantic Scholar 请求。底层客户端仍把任何更大的输入切成最多 500 个 ID 的批次；数据库写入仍保持每批 100 条，Crossref 路径上的 OpenAlex DOI 增强也仍保持每批最多 100 条。官方 Graph API schema 将 batch 上限定为 500；官方产品页给出的 introductory API-key 限额为 1 request/second：

- [Semantic Scholar API](https://www.semanticscholar.org/product/api)
- [Graph API OpenAPI](https://api.semanticscholar.org/graph/v1/swagger.json)

多个索引进程共享保守的时隙模型：

- 基础间隔为 1 秒。
- worker 初始偏移为 `worker_id × 1 秒`。
- 同一 worker 的后续间隔为 `process_count × 1 秒`。
- 每次请求使用配置池中的第一个 key。

这让默认多进程执行近似共享 1 RPS，但不是跨主机的分布式限流器。页级 DOI 合并只减少占用的串行时隙，不改变进程数、请求并发或时隙间隔。

主要增强字段：

| 目标字段      | 来源                |
| ------------- | ------------------- |
| DOI 归并      | `externalIds.DOI`   |
| 摘要 fallback | `abstract`          |
| OA 状态       | `isOpenAccess`      |
| PDF 候选      | `openAccessPdf.url` |

Semantic Scholar 返回 “No valid paper ids given” 时，该批按空结果处理；其他不可接受的 4xx 会使索引失败。

## 字段优先级

| 落库字段           | 当前优先级                                               |
| ------------------ | -------------------------------------------------------- |
| `abstract`         | Crossref → OpenAlex → Semantic Scholar                   |
| `open_access`      | Semantic Scholar 或 OpenAlex 任一标记为 OA 即为 true     |
| `full_text_file`   | Semantic Scholar PDF → OpenAlex PDF → OpenAlex OA 落地页 |
| `content_location` | OpenAlex OA 落地页 → Crossref URL → DOI URL              |
| `permalink`        | DOI URL → `content_location`                             |
| `title`            | Crossref → OpenAlex                                      |
| `date`             | Crossref 日期链 → OpenAlex `publication_date`            |
| `authors`          | Crossref → OpenAlex                                      |

`full_text_file` 表示上游提供的 OA 候选，不代表机构订阅授权。展示层仍应通过文章访问接口判断动作。

## 重试、并发与可观测性

Scholarly HTTP 请求默认最多执行 3 次：

- 传输错误会重试。
- `429`、`500`、`502`、`503`、`504` 会重试。
- 两次退避分别为 1 秒和 2 秒。
- 其他非 2xx 响应直接失败。

`--processes` 控制同一 CSV 内的期刊 worker 数；CSV 文件之间仍串行。单个 worker 内，Crossref、OpenAlex 与 Semantic Scholar 请求均为串行，`--workers` 不会扩大 scholarly 请求并发。

每次请求的服务、端点、脱敏 URL、状态码、重试与成功状态都会汇总到索引运行统计；API key 不进入调试输出或统计 URL。
