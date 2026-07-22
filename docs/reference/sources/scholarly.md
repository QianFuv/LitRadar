# Scholarly Provider

Scholarly 是内置 Provider adapter，不是内容 schema。它把 Crossref、OpenAlex 和 Semantic Scholar 响应转换为[规范 Provider 契约](../index-provider-contract.md)，并可独立提供在线摘要页能力。

## 能力声明

| 注册                            | 能力                      | 进程边界与不提供项            |
| ------------------------------- | ------------------------- | ----------------------------- |
| `scholarly_index_registration`  | `IndexContentProvider`    | `index` 进程；不提供在线动作  |
| `scholarly_access_registration` | `ArticleAbstractProvider` | `serve` API；不提供索引或全文 |

索引进程和 API 进程分别构造所需注册，管理端按相同逻辑名称把两者聚合为 `index_content + article_abstract`。这就是“分进程注册”：同一二进制内的命令边界不同，不是两个常驻服务，也不是自动 fallback。索引能力不会让文章记录携带 `scholarly` provenance；在线能力也不要求文章曾由 Scholarly 索引。

## 索引上游职责

| 上游             | 请求时职责                                                | 可进入规范内容的字段                            |
| ---------------- | --------------------------------------------------------- | ----------------------------------------------- |
| Crossref         | 按 ISSN 获取主文章清单                                    | DOI、题名、作者、摘要、日期、卷期页码、撤稿关系 |
| OpenAlex         | DOI 增强；Crossref 全部 404 时解析期刊并提供清单 fallback | 题名、作者、摘要、日期、PMID、OA                |
| Semantic Scholar | 按 DOI 批量增强                                           | 摘要、OA                                        |

上游 URL、source ID、Crossref cursor、OpenAlex cursor 和 Semantic Scholar PDF/landing-page URL 不进入 `ArticleDraft` 或内容数据库。URL 只允许存在于私有 transport payload 和当前调用内。

只要选中的目录被 `index_provider_routes` 路由到 `scholarly`，`openalex_api_key_pool`、`semantic_scholar_api_key_pool` 和 `crossref_mailto_pool` 都必须至少包含一个非空值。缺少任一类会在创建内容库、控制库或其他索引状态前失败。配置见[运行配置](../configuration.md)。

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

| 规范字段                          | 顺序/规则                                                            |
| --------------------------------- | -------------------------------------------------------------------- |
| `title`                           | Crossref，缺失时 OpenAlex                                            |
| `authors`                         | Crossref，缺失时 OpenAlex；只保留有序 display name                   |
| `abstract_text`                   | Crossref 去标记文本，缺失时 OpenAlex，再缺失时 Semantic Scholar      |
| `publication_year` / `date`       | Crossref 日期链，缺失时 OpenAlex publication date                    |
| `volume` / `issue_number` / pages | Crossref                                                             |
| `doi`                             | 规范化为小写标识符，不保存 DOI URL                                   |
| `pmid`                            | OpenAlex `ids.pmid` 的数字形式                                       |
| `open_access`                     | Semantic Scholar 或 OpenAlex 任一明确为 OA 时为 true                 |
| `retraction_dois`                 | Crossref `updated-by` 中 type 为 retraction 的全部规范 DOI，排序去重 |

Provider 不返回 PDF URL、landing page、permalink 或 content location。在线全文不是 Scholarly 当前声明的能力。

通用 Crossref `relation` 不表示撤稿，不能填充 `retraction_dois`。`updated-by` 中 correction 等其他 update type、格式不合法的 DOI、source 标签、更新时间和原始 update payload 都会被忽略；多个来源重复报告同一撤稿 DOI 时只保留一条。

## Crossref 分页

- 基础地址：`https://api.crossref.org/v1`；
- `type:journal-article`；
- `rows=225`；
- `sort=published&order=asc`；
- 从 `cursor=*` 开始并使用 `message.next-cursor`；
- 少于 225 条或没有下一 cursor 时结束。

Crossref cursor 只作为 Provider checkpoint 内容存于 `data/index-control`，不会进入内容库。

当前 [Crossref REST API 访问合同](https://www.crossref.org/documentation/retrieve-metadata/rest-api/access-and-authentication/) 的 polite pool 为 `10 req/s`、并发 `3`。Scholarly 对整个父进程树使用一个公共 epoch，每 110 ms 允许一个请求尝试，约为 `9.09 req/s`；最多三个期刊子进程各有一个请求在途。每次重试也必须取得下一个未来相位，错过的相位不会补发成突发流量。

mailto 是 Crossref 的联系身份，不是独立配额凭据。客户端稳定使用池中的第一个 mailto；一个和三个 mailto 得到完全相同的速率/并发预算，不轮转身份来放大容量。

## OpenAlex fallback 与已知限制

OpenAlex `/sources` 以 ISSN 精确查询优先，题名 search 只作为 fallback。source works 使用 `primary_location.source.id`、cursor 和出版日期升序。

当前 [OpenAlex 认证与计费合同](https://developers.openalex.org/api-reference/authentication) 为每个 API key 最多 `100 req/s`，并为每个 key 独立统计每日 credits。Scholarly 为每个健康 key 建立跨进程公共相位：每 11 ms 一个相位，约为 `90.9 req/s/key`。进程 `p` 拥有 `epoch + p × 11 ms + n × process_count × 11 ms` 的相位；改变进程数只改变所有权，不改变单 key 或 key 池的总速率。

所有配置的 OpenAlex key 都参与调度。选择会考虑剩余 credits、在途请求、冷却和认证状态；401/403 只禁用对应 slot，429/reset 只冷却对应 slot，失败切换不能绕过另一个 key 的未来相位。调度器解析 remaining、reset 和单次 credits-used，并保留 `workers × processes × 最大已知单次 cost` 的每日 headroom；额度未知时每个 key/进程只允许一个探测请求。每个进程最多六个 OpenAlex DOI 子批在途，三个进程的全局上限为 18。

[OpenAlex deprecation 说明](https://developers.openalex.org/guides/deprecations)记录其自 2026 年 2 月起忽略 mailto。LitRadar 的 source、source search、source works 和 DOI 请求均不发送 Crossref mailto，URL 长度预算也只计入 OpenAlex key。

当前 source-works fallback 仍请求 `per-page=200`，而现有上游文档的公开上限是 100。这只影响 Crossref 对全部 ISSN 返回 404 后的 OpenAlex source 清单路径。代码任务修复该偏差时必须同时调整分页终止条件和 fixtures；本文不把 200 描述为受上游保证的值。

## Semantic Scholar 节流

请求为 `POST /graph/v1/paper/batch`，最多 500 个规范 DOI ID。当前 [Semantic Scholar API 合同](https://www.semanticscholar.org/product/api) 的入门配额为每 API key `1 req/s`。Scholarly 对每个合法 key 使用 1,100-ms 跨进程相位，约为 `0.909 req/s/key`；生产路径会把更小的内部间隔钳制到 1,100 ms。

key `k`、进程 `p` 的相位为 `epoch + p × 1,100 ms + k × 1,100 ms / key_count + n × process_count × 1,100 ms`。key 间在一个周期内均匀错开，使串行 batch 调用也能使用两个或三个独立 key 的容量；对任一 key，全部进程合并后仍至少间隔 1,100 ms。401/403 只禁用被选 key，429 使用 Retry-After 与退避的较大值冷却被选 key，5xx/传输失败可切换到其他健康 key，但每次尝试仍需自己的未来相位。

这些相位只协调同一条 `litradar index` 命令创建的进程树，不是跨命令、跨主机或跨应用的分布式限流器。key 的认证/冷却观测保守地保存在各子进程中，因此另一个子进程可能需要独立观察同一失效响应；公共相位仍保证它们不会叠加超过每 key 的本地计划速率。其他客户端共享同一 key 或上游临时降额时仍可能产生 429；调用方应把它视为外部协调信号，而不是通过更激进重试绕过。

“No valid paper ids given” 按空增强处理；其他不接受的 4xx 明确失败。

## 在线摘要页

Scholarly 在线 adapter 不请求或读取索引时保存的 URL：

1. `ArticleLocator` 有 DOI 时，生成当前请求的 `https://doi.org/{doi}`；
2. 否则有 PMID 时，生成 `https://pubmed.ncbi.nlm.nih.gov/{pmid}/`；
3. 两者都没有时返回 `NotFound`。

该摘要能力使用上述规范目的地。注册的精确 allowlist 只有 `doi.org` 和 `pubmed.ncbi.nlm.nih.gov`；API 再执行统一 HTTPS/host 校验并返回 no-store 307。生成 URL 不写回数据库。前端文章详情弹窗展示本地已存元数据，不调用该 adapter。

默认摘要顺序中的 `scholarly → cnki` 是请求时 fallback：有 DOI/PMID 时 scholarly 通常先返回，否则或解析失败时继续 CNKI。管理员可以按 CSV/database stem 继承默认顺序、完整替换顺序或用空列表禁用；这不改变 `index_provider_routes`。

## 重试、日志与秘密

Crossref journal-list GET 收到 HTTP 响应后仍最多尝试三次；`429/500/502/503/504` 沿用 1/2 秒退避，其他非 2xx 直接失败。只有 `Client::execute` 没有产生任何 HTTP 响应的传输失败可以扩展到最多六次，并按 1/2/4/8/16 秒退避。扩展次数依据请求 timeout 选择，新增尝试的模型包络不得超过 180 秒；默认 20 秒 timeout 选择六次和 151 秒包络，较长 timeout 会降为五次、四次或原有三次，但不会低于三次。OpenAlex 和 Semantic Scholar 为了在 key 故障时完成合法 failover，单个逻辑请求最多尝试 `key_count + 2` 次；本次验证覆盖 `1..=3` 个 key，因此该范围最多五次。每个网络尝试（包括 retry）都计入被选 Provider/key 的相位。401/403 只停用被选 key。

每次逻辑请求的成功、失败和 retry 会汇总到 `index.provider.attempts` 结构化终态事件。OpenAlex/Semantic Scholar 尝试事件只增加安全的 key-slot 编号、状态分类、retry 标志和耗时，不记录 key 值或请求体。内容库没有 API call/statistics 表。Crossref 无响应传输失败在尝试记录和返回错误中都固定为 `transport failure`，不保留可能携带 URL 或查询参数的 Reqwest 原始错误。API key、完整查询秘密、DOI 请求体、响应正文和上游 URL 不进入安全错误或持久状态；Semantic Scholar 非白名单错误正文会折叠为固定消息。

调度器暴露的是有安全余量的可用容量，不是吞吐保证。实际吞吐近似受 `min(Provider 预算, 在途容量 / 响应延迟, 产生工作速率)` 限制；低 worker、慢响应或工作不足不能被标记为限流器利用率不足，也不承诺精确 100% 使用或任何外部状态下都零 429。

## 维护测试

修改 adapter 时至少覆盖：

- Crossref cursor、多 ISSN 404 和 OpenAlex fallback；
- OpenAlex DOI 批量去重、source 匹配和 undated 请求；
- Semantic Scholar 500 ID 分批、节流和错误分类；
- 不同上游 payload 产生相同规范文章；
- 规范 batch 中没有 Provider/source/URL 字段；
- DOI/PMID 在线动作、缺失标识和 host allowlist；
- checkpoint 重放不复制内容或改变 ID。
