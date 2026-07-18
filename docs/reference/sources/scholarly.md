# Scholarly Provider

Scholarly 是内置 Provider adapter，不是内容 schema。它把 Crossref、OpenAlex 和 Semantic Scholar 响应转换为[规范 Provider 契约](../index-provider-contract.md)，并可独立提供在线详情/摘要页能力。

## 能力声明

| 注册                            | 能力                                               | 不提供     |
| ------------------------------- | -------------------------------------------------- | ---------- |
| `scholarly_index_registration`  | `IndexContentProvider`                             | 在线动作   |
| `scholarly_access_registration` | `ArticleDetailProvider`、`ArticleAbstractProvider` | 索引、全文 |

索引进程和 API 进程分别构造所需注册。索引能力不会让文章记录携带 `scholarly` provenance；在线能力也不要求文章曾由 Scholarly 索引。

## 索引上游职责

| 上游             | 请求时职责                                                | 可进入规范内容的字段                            |
| ---------------- | --------------------------------------------------------- | ----------------------------------------------- |
| Crossref         | 按 ISSN 获取主文章清单                                    | DOI、题名、作者、摘要、日期、卷期页码、撤稿关系 |
| OpenAlex         | DOI 增强；Crossref 全部 404 时解析期刊并提供清单 fallback | 题名、作者、摘要、日期、PMID、OA                |
| Semantic Scholar | 按 DOI 批量增强                                           | 摘要、OA                                        |

上游 URL、source ID、Crossref cursor、OpenAlex cursor 和 Semantic Scholar PDF/landing-page URL 不进入 `ArticleDraft` 或内容数据库。URL 只允许存在于私有 transport payload 和当前调用内。

被 `index_provider_routes` 路由到 `scholarly` 的目录需要非空 `openalex_api_key_pool` 和 `semantic_scholar_api_key_pool`。`crossref_mailto_pool` 可选，生产环境应配置可联系邮箱。配置见[运行配置](../configuration.md)。

## 索引流程

对每个 `JournalCatalogEntry`：

1. 按维护的 `issn`、`eissn`、`all_issns` 构造去重候选。
2. 依次请求 Crossref `/journals/{issn}/works`；第一个非 404 响应成为主清单。
3. 只有全部 ISSN 都返回 404 时，才按 ISSN、再按维护标题/别名解析 OpenAlex source，并读取 source works。
4. 对当前页 DOI 规范化和去重，按最多 100 个 DOI 请求 OpenAlex 增强，并按最多 500 个 DOI 请求 Semantic Scholar batch。
5. 把上游变体映射到 `JournalDraft`、`IssueDraft` 和 `ArticleDraft`。
6. 返回一页 `ProviderBatch`；下一页 cursor 编码为 opaque checkpoint，由控制库保存。

Crossref 成功但结果为空不会触发 OpenAlex source fallback。没有 DOI 的记录仍可在具备充分 bibliographic identity 时进入内容库，但不会进入 DOI 增强。

## 字段合并

| 规范字段                          | 顺序/规则                                                       |
| --------------------------------- | --------------------------------------------------------------- |
| `title`                           | Crossref，缺失时 OpenAlex                                       |
| `authors`                         | Crossref，缺失时 OpenAlex；只保留有序 display name              |
| `abstract_text`                   | Crossref 去标记文本，缺失时 OpenAlex，再缺失时 Semantic Scholar |
| `publication_year` / `date`       | Crossref 日期链，缺失时 OpenAlex publication date               |
| `volume` / `issue_number` / pages | Crossref                                                        |
| `doi`                             | 规范化为小写标识符，不保存 DOI URL                              |
| `pmid`                            | OpenAlex `ids.pmid` 的数字形式                                  |
| `open_access`                     | Semantic Scholar 或 OpenAlex 任一明确为 OA 时为 true            |
| `retraction_doi`                  | Crossref relation 中的规范 DOI                                  |

Provider 不返回 PDF URL、landing page、permalink 或 content location。在线全文不是 Scholarly 当前声明的能力。

## Crossref 分页

- 基础地址：`https://api.crossref.org/v1`；
- `type:journal-article`；
- `rows=225`；
- `sort=published&order=asc`；
- 从 `cursor=*` 开始并使用 `message.next-cursor`；
- 少于 225 条或没有下一 cursor 时结束。

Crossref cursor 只作为 Provider checkpoint 内容存于 `data/index-control`，不会进入内容库。

## OpenAlex fallback 与已知限制

OpenAlex `/sources` 以 ISSN 精确查询优先，题名 search 只作为 fallback。source works 使用 `primary_location.source.id`、cursor 和出版日期升序。

当前 source-works fallback 仍请求 `per-page=200`，而现有上游文档的公开上限是 100。这只影响 Crossref 对全部 ISSN 返回 404 后的 OpenAlex source 清单路径。代码任务修复该偏差时必须同时调整分页终止条件和 fixtures；本文不把 200 描述为受上游保证的值。

## Semantic Scholar 节流

请求为 `POST /graph/v1/paper/batch`，最多 500 个规范 DOI ID。多个本机索引进程使用保守时隙：基础间隔 1 秒，worker 初始偏移为 `worker_id × 1s`，同一 worker 后续间隔为 `process_count × 1s`。这不是跨主机分布式限流器。

“No valid paper ids given” 按空增强处理；其他不接受的 4xx 明确失败。

## 在线详情和摘要页

Scholarly 在线 adapter 不请求或读取索引时保存的 URL：

1. `ArticleLocator` 有 DOI 时，生成当前请求的 `https://doi.org/{doi}`；
2. 否则有 PMID 时，生成 `https://pubmed.ncbi.nlm.nih.gov/{pmid}/`；
3. 两者都没有时返回 `NotFound`。

详情和摘要页当前使用同一规范目的地。注册的精确 allowlist 只有 `doi.org` 和 `pubmed.ncbi.nlm.nih.gov`；API 再执行统一 HTTPS/host 校验并返回 no-store 307。生成 URL 不写回数据库。

## 重试、日志与秘密

Scholarly HTTP 请求默认最多三次。传输错误及 `429/500/502/503/504` 重试，两次退避为 1 秒和 2 秒；其他非 2xx 直接失败。

每次逻辑请求的成功、失败和 retry 会汇总到 `index.provider.attempts` 结构化终态事件。内容库没有 API call/statistics 表。API key、完整查询秘密、响应正文和上游 URL 不进入安全错误或持久状态。

## 维护测试

修改 adapter 时至少覆盖：

- Crossref cursor、多 ISSN 404 和 OpenAlex fallback；
- OpenAlex DOI 批量去重、source 匹配和 undated 请求；
- Semantic Scholar 500 ID 分批、节流和错误分类；
- 不同上游 payload 产生相同规范文章；
- 规范 batch 中没有 Provider/source/URL 字段；
- DOI/PMID 在线动作、缺失标识和 host allowlist；
- checkpoint 重放不复制内容或改变 ID。
