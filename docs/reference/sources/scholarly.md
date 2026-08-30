# Scholarly Provider

Scholarly 是内置 Provider adapter，不是内容 schema。它把 Crossref、OpenAlex 和 Semantic Scholar 响应转换为[规范 Provider 契约](../index-provider-contract.md)，并可独立提供在线摘要页能力。

## 能力声明

| 注册                            | 能力                      | 进程边界与不提供项            |
| ------------------------------- | ------------------------- | ----------------------------- |
| `scholarly_index_registration`  | `IndexContentProvider`    | `index` 进程；不提供在线动作  |
| `scholarly_access_registration` | `ArticleAbstractProvider` | `serve` API；不提供索引或全文 |

索引进程和 API 进程分别构造所需注册，管理端按相同逻辑名称把两者聚合为 `index_content + article_abstract`。这就是“分进程注册”：同一二进制内的命令边界不同，不是两个常驻服务，也不是自动 fallback。索引能力不会让文章记录携带 `scholarly` provenance；在线能力也不要求文章曾由 Scholarly 索引。

## 托管代理归属

[运行配置](../configuration.md)中的 `scholarly` 代理开关只覆盖索引 adapter 发出的 Crossref、OpenAlex 和 Semantic Scholar HTTP，包括 source 查找、分页、DOI/batch 增强、重试和 Provider-local fallback。关闭时这些 client 明确忽略系统代理变量并受管直连；打开时只使用共用 `provider_proxy_url`，代理失败不会改成直连。

在线摘要 adapter 不发出 HTTP：它只根据已有 DOI 或 PMID 在本地生成受 host allowlist 约束的 HTTPS redirect，再由浏览器访问目的地。因此 `scholarly` 开关不代理该 redirect，也不影响 AI、通知/PushPlus 或其他非 Provider client。多进程索引通过 protocol-v8 stdin bootstrap 把代理 URL、OpenAlex/Semantic Scholar key pool 和 Crossref mailto pool 一次性传给对应 worker；持久 request JSON、参数、环境、日志和 Debug 均不包含这些值。Scholarly worker 还从 bootstrap 接收 core 构造的工作集根路径，其他 Provider 不接收该字段。启动新 worker 前只会清理文件名与 JSON 元数据一致、早于索引 lease 且协议版本低于 8 的普通遗留 request 文件；新文件、无效文件和链接不会被删除。

## 索引上游职责

| 上游             | 请求时职责                                                | 可进入规范内容的字段                            |
| ---------------- | --------------------------------------------------------- | ----------------------------------------------- |
| Crossref         | 按 ISSN 获取主文章清单                                    | DOI、题名、作者、摘要、日期、卷期页码、撤稿关系 |
| OpenAlex         | DOI 增强；Crossref 整刊查询均为 404 或空结果时提供清单 fallback | 题名、作者、摘要、日期、PMID、OA                |
| Semantic Scholar | 按 DOI 批量增强                                           | 摘要、OA                                        |

上游 URL、source ID、Crossref cursor、OpenAlex cursor 和 Semantic Scholar PDF/landing-page URL 不进入 `ArticleDraft` 或内容数据库。OpenAlex source ID 与 cursor 只存在于可丢弃 traversal checkpoint；Crossref cursor 可存在于 traversal 和私有工作集。成功 anchor 只使用规范书目信息和日期，不含 Provider/upstream ID 或 URL。

只要选中的目录被 `index_provider_routes` 路由到 `scholarly`，`openalex_api_key_pool`、`semantic_scholar_api_key_pool` 和 `crossref_mailto_pool` 都必须至少包含一个非空值。缺少任一类会在创建内容库、控制库或其他索引状态前失败。配置见[运行配置](../configuration.md)。

## 索引流程

对每个 `JournalCatalogEntry`：

1. 按维护的 `issn`、`eissn`、`all_issns` 构造去重候选。
2. 根据 `IndexFetchContext` 建立 Bootstrap、Incremental 或 FullRescan 窗口；Incremental 冻结 committed issue anchor 和日期下界，Crossref 另冻结本次遍历的 UTC 整秒上界 `T`。
3. 依次探测 Crossref `/journals/{issn}/works` 的完整创建日期范围；整刊 404 或为空时尝试下一个 ISSN。按下述 created 分片规则收集，单步最多消费一个响应，先返回不含文章的 Continue 供 core 确认进度。
4. 全部 ISSN 均无可用 Crossref 清单时，按 ISSN、再按维护标题/别名解析 OpenAlex source，并沿用其出版日期降序分页。
5. Crossref 的分片、父子总数和全局唯一数全部通过后，才按本地期次组排序，冻结 candidate 并选择完整的 candidate/base 窗口。
6. 仅对选中的本地输出页规范化 DOI，按最多 100 个 DOI 请求 OpenAlex 增强，并按最多 500 个 DOI 请求 Semantic Scholar batch；映射为 `JournalDraft`、`IssueDraft` 和 `ArticleDraft`。
7. 返回有界 `ProviderBatch`。Crossref Continue 保存收集状态或本地 keyset 位置；只有完整输出所选期次后才 Complete，由 core 在内容提交后保存成功 anchor。

空的创建日期子分片只表示该片完成，不能触发整刊 fallback 或推进 anchor。已有 anchor 的增量候选为空或找不到 base 时，仍保留同源无 update 过滤重放语义，避免把暂时没有更新误判为需要切换主清单。没有 DOI 的记录仍可在具备充分 bibliographic identity 时进入内容库，但不会进入 DOI 增强。

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

[2026-08 官方公告](https://community.crossref.org/t/changes-to-cursors-filtering-and-sorting-in-the-rest-api/16246)及其 [8 月 24 日上线确认](https://community.crossref.org/t/changes-to-cursors-filtering-and-sorting-in-the-rest-api/16246/4)说明：新 cursor 不再过期，也不再固定结果集；`cursor` 与 `published` 等出版日期排序组合会被拒绝，每次续页必须重复相同查询参数。旧文档中 `sort=published&cursor=*` 或五分钟过期示例不再作为当前合同依据。

LitRadar 采用官方建议的“小窗口、created 条件、小结果单响应、核对首响应总数、大结果无排序后本地整理”。保留 update 条件并遍历完整 created 历史，符合[官方技术团队的组合过滤建议](https://community.crossref.org/t/date-range-search-of-index-changes-seems-to-retrieve-too-many-records/1468/8)；自动二分、去重和分片树守卫是本项目的实现方式。

所有请求使用 `https://api.crossref.org/v1/journals/{issn}/works` 和 `type:journal-article`：

| 步骤 | 查询与完成条件 |
| ---- | -------------- |
| 创建日期定界 | 无 cursor，`rows=1&sort=created&order=asc`，只加 `until-created-date:T`，不加 update 条件；用最早记录的 `created.timestamp` 毫秒值转换为 UTC 秒下界 `C`。 |
| 普通分片 | 查询 `[C,T]` 或其子区间，无 cursor、无 sort/order、`rows=225`。响应条数必须等于 `min(message.total-results,225)`；总数不超过 225 时，实际条数和唯一数必须都与总数相等。 |
| 大于 225 的分片 | 丢弃探测响应的文章前缀，将 UTC 整秒闭区间 `[a,b]` 二分为 `[a,m]`、`[m+1,b]`；继续单响应探测，最大深度 64。 |
| 单秒仍大于 225 | 该片从 `cursor=*&rows=225` 完整重取，无 sort/order；后续只改变 cursor，其他参数固定，不使用 offset。 |
| 完整性校验 | 每片累计条数、唯一数和首响应总数一致；每页总数不得漂移，父片等于子片之和，根总数等于全局唯一数。游标最后一页恰好满 225 条时继续确认终止响应，不能仅凭已达到总数提前完成。 |

225 条沿用既有的单次请求规模。[官方允许的最大 rows 为 1000](https://github.com/CrossRef/rest-api-doc#rows)，无需使用最大值才能采用单响应与计数校验方案。较小阈值可能增加分片和请求数，不新增 `rows=0` 探测或超限回退。已验证的旧版完整叶片可以继续复用，已有游标仍以相同的 225 条参数恢复；新产生的 226–1000 条单秒游标状态不能交给仍要求总数大于 1000 的旧二进制恢复。

日期边界采用官方支持的[包含式 UTC 秒精度](https://community.crossref.org/t/query-the-rest-api-with-hour-minute-second-resolution/13821)。有界 Incremental 在每个分片附加相同的 `from-update-date:<anchor 年份的 1 月 1 日>` 和 `until-update-date:T`，创建日期则始终覆盖整刊历史。因此早年创建、最近修改的 DOI 不会因创建年份早于更新下界而被排除。无 anchor、FullRescan 或同源无界重放去掉全部 update 条件，只保留完整 created 范围和同一 `T`；无界重放还保留已经冻结的 candidate。

重复、短页与计数矛盾或记录越界时，未发布的叶片最多重取一次，保留其他已验证片；父子/全局计数漂移最多重建一代。预算持久化，重试不改变 `C/T`，持续异常明确失败，不推进成功 anchor。普通 HTTP/transport 重试仍按后文的有限预算处理，HTTP 500 不再触发更换 cursor，240 秒过期重扫已移除。

created 不变只能稳定分片归属，不能冻结文章字段或 update 条件的成员资格。跨出固定 update 上限、等量替换、journal/type 变更和延迟入索引仍可能影响结果，部分情况无法由计数发现。固定 cursor、`T`、本地缓存和计数相等都不是远端快照保证；本流程也不会因为远端本次未返回某篇文章而删除已有内容。

### 本地工作集与排序

工作集位于 core 提供的 `data/index-work/scholarly/`，与内容库、控制库、Git、内容发现和备份分离。随机 token 只定位当前 catalog 和冻结 context 的普通文件；打开和清理前校验归属、路径、symlink/reparse point，不自动删除未知或其他期刊的文件。工作集仅保存消费所需的书目字段、created 时间及进度，不保存凭据、mailto、代理、references 或上游资源 URL。

收集页和分片状态在同一 SQLite 事务内保存，验证完成后结果固定。相同期次先聚合为连续组：按组内最大出版日期降序，再按可用年份、数值卷期降序和 fingerprint 字节序排序；组内按日期降序、记录 key 升序。缺失月/日只在排序键中补 01，不改变内容日期精度，无日期记录排后。这个顺序由本地规则确定，不声称复刻上游旧排序的隐含平局规则。

查询通过持续维护的索引和 keyset 分页，每个输出页最多 225 条、16 MiB payload，不构造整刊 Vec，也不依赖 `/tmp` 大排序文件。工作集使用 4 MiB SQLite page cache、关闭 mmap，主文件上限 4 GiB；page cache 不是进程 RSS 上限，事务日志还需要额外磁盘空间。HTTP 响应仍限 16 MiB，225 条探测也受此限制；单条元数据大小不固定，较少条数不保证永不超限。磁盘满、容量或响应超限都会失败并保留正式内容和旧成功 anchor，不截断结果凑数。

core checkpoint 是确认进度的权威。缓存超前一页时先重放该已暂存步骤；内容已提交而控制事务失败时依靠既有 identity/upsert 重放。工作集缺失或可识别的自有文件损坏时，从同一 `C/T`、update 条件和 candidate 重建，不继续缺少前缀的旧 cursor。Provider 准备返回 Complete 时可清理缓存；若随后 core 提交失败，恢复仍按缺失缓存规则重取。路径或身份不匹配直接失败。

### 请求预算

当前 [Crossref REST API 访问合同](https://www.crossref.org/documentation/retrieve-metadata/rest-api/access-and-authentication/) 的 polite pool 为 `10 req/s`、并发 `3`。Scholarly 对整个父进程树使用一个公共 epoch，每 110 ms 允许一个请求尝试，约为 `9.09 req/s`；最多三个期刊子进程各有一个请求在途。每次重试也必须取得下一个未来相位，错过的相位不会补发成突发流量。

mailto 是 Crossref 的联系身份，不是独立配额凭据。客户端稳定使用池中的第一个 mailto；一个和三个 mailto 得到完全相同的速率/并发预算，不轮转身份来放大容量。

## OpenAlex fallback 与已知限制

OpenAlex `/sources` 以 ISSN 精确查询优先，题名 search 只作为 fallback。source works 使用 `primary_location.source.id`、`type:article|book-chapter`、cursor 和 `publication_date:desc`；`book-chapter` 覆盖以 ISSN 编目的连续出版物，`book`、`paratext`、`other` 和 `editorial` 仍被排除。Incremental 每页固定携带 anchor 的 `from_created_date` 下界。

某些 OpenAlex 套餐拒绝 `from_created_date`。客户端只对明确的 plan-restriction 错误启用一次 Provider-local fallback：清除日期 filter 和旧 query cursor，从 source 头部重放；核心模式和控制协议不变化。普通 429 仍然失败，不会被误判为套餐 fallback。

当前 [OpenAlex 认证与计费合同](https://developers.openalex.org/api-reference/authentication) 为每个 API key 最多 `100 req/s`，并为每个 key 独立统计每日 credits。Scholarly 为每个健康 key 建立跨进程公共相位：每 11 ms 一个相位，约为 `90.9 req/s/key`。进程 `p` 拥有 `epoch + p × 11 ms + n × process_count × 11 ms` 的相位；改变进程数只改变所有权，不改变单 key 或 key 池的总速率。

所有配置的 OpenAlex key 都参与调度。选择会考虑剩余 credits、在途请求、冷却和认证状态；401/403 只禁用对应 slot，429/reset 只冷却对应 slot，失败切换不能绕过另一个 key 的未来相位。调度器解析 remaining、reset 和单次 credits-used，并保留 `workers × processes × 最大已知单次 cost` 的每日 headroom；额度未知时每个 key/进程只允许一个探测请求。每个进程最多六个 OpenAlex DOI 子批在途，三个进程的全局上限为 18。

[OpenAlex deprecation 说明](https://developers.openalex.org/guides/deprecations)记录其自 2026 年 2 月起忽略 mailto。LitRadar 的 source、source search、source works 和 DOI 请求均不发送 Crossref mailto，URL 长度预算也只计入 OpenAlex key。

当前 source-works fallback 仍请求 `per-page=200`，而现有上游文档的公开上限是 100。这只影响 Crossref 对全部 ISSN 均无可用清单后的 OpenAlex source 清单路径。代码任务修复该偏差时必须同时调整分页终止条件和 fixtures；本文不把 200 描述为受上游保证的值。

## Issue anchor、fallback 与能力边界

Scholarly anchor v1 使用 Provider 私有 JSON，保存规范 issue fingerprint 和可用的 `from_sync_date`。fingerprint 与内容 issue identity 的优先级一致：

1. publication year + 规范化 volume/issue（至少一个存在）；
2. 规范日期；
3. 规范化 issue title，可带 publication year。

Crossref Incremental 在整个工作集验证后，才从本地首个有效期次组冻结 candidate head。输出范围包含 candidate 与 base 的组内最大日期之间的全部期次组，也完整包含两端同日平局的其他组；只有最后一个选中组全部输出才 Complete。按组内最大日期严格晚于冻结 candidate 的新组不参与本次输出。base 缺失、candidate 可证明旧于 base、日期区间倒置或 fingerprint 不足以确定边界时，切换同源无 update 条件的完整收集。无界模式保留冻结 candidate，重取后 candidate 消失则失败。

OpenAlex 继续使用原有有序分页与整期边界停止规则；套餐拒绝日期 filter 时保留既有无过滤重放。两条路径的 next anchor 都取冻结 candidate，不因恢复时出现新 head 而重新计算。日期 filter 不能替代期次边界校验。

新 traversal checkpoint 为 v2，保存冻结窗口、Crossref token/收集阶段/计数或本地输出位置，仍受 65,536 字节上限约束。严格读取有效 v1：旧 Crossref cursor 被丢弃，在原 base/candidate 约束下建立新查询；旧 OpenAlex cursor 和既有恢复语义保留。未知版本、无版本或损坏状态、mode/base 不匹配直接拒绝。成功 anchor 仍为 v1，内容 v6、catalog control v4、batch ledger v2 和 Provider contract v3 均不变；无需清空正式数据库。开始写入 v2 后，不应直接用不能识别它的旧二进制恢复。

同次中断恢复可复用已验证片和当前有效 cursor，不再因 240 秒或 HTTP 500 重扫。下一次独立 `--update` 必须新建查询和 `T`，保留成功边界整期补查，才能收录上次期次后来追加的文章；终点 cursor 不是永久变更水位。当前规则没有固定回看 7 天或 30 天的窗口。

这条边界增量能发现新期次和成功边界期次后续追加的文章，但不保证发现更早历史期次的补录，也不保证刷新边界以前文章的摘要、OA、作者或撤稿关系。需要核对历史回填和旧元数据时运行 `--full-rescan`；该模式扫描完整 source 历史且不生成 changes JSON。

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

Crossref、OpenAlex 和 Semantic Scholar 共用的 HTTP client 禁止自动重定向。任何 3xx 都在原始上游响应处失败，不会访问 `Location`，也不会把 Semantic Scholar `x-api-key`、DOI batch body 或查询参数转发到其他 origin/协议。

所有 Scholarly JSON 响应的解压后上限为 16 MiB。读取先检查可用的 Content-Length，再对透明解压后流最多保留 `limit + 1` 字节，所以 chunked 响应或小 gzip/大解压正文也会明确失败。超限不参与 transport retry 或 key failover，也不保留响应正文。

每次逻辑请求的成功、失败和 retry 会汇总到 `index.provider.attempts` 结构化终态事件。OpenAlex/Semantic Scholar 尝试事件只增加安全的 key-slot 编号、状态分类、retry 标志和耗时，不记录 key 值或请求体。内容库没有 API call/statistics 表。Crossref 无响应传输失败在尝试记录和返回错误中都固定为 `transport failure`，不保留可能携带 URL 或查询参数的 Reqwest 原始错误。API key、完整查询秘密、DOI 请求体、响应正文和上游 URL 不进入安全错误或持久状态；Semantic Scholar 非白名单错误正文会折叠为固定消息。

调度器暴露的是有安全余量的可用容量，不是吞吐保证。实际吞吐近似受 `min(Provider 预算, 在途容量 / 响应延迟, 产生工作速率)` 限制；低 worker、慢响应或工作不足不能被标记为限流器利用率不足，也不承诺精确 100% 使用或任何外部状态下都零 429。

## 维护测试

修改 adapter 时至少覆盖：

- Crossref created/update 组合、UTC 包含式秒分界、0/224/225/226/1000/1001 条与空子片、单次响应大小保护、多 ISSN 404 和 OpenAlex fallback；
- 10,001/100,001 条分散创建日期或集中单秒的完整收集，去重、父子/根计数和持久重试预算；
- 本地乱序期次归并、同日平局、整期 base 补查，以及下一次 update 新增的边界文章；
- Crossref/OpenAlex 规范字段产生相同 issue fingerprint，anchor 不含上游 ID；
- missing-base 无过滤重放和 OpenAlex 后续页套餐 fallback 保持冻结 candidate；
- OpenAlex DOI 批量去重、source 匹配和 undated 请求；
- Semantic Scholar 500 ID 分批、节流和错误分类；
- 不同上游 payload 产生相同规范文章；
- 规范 batch 中没有 Provider/source/URL 字段；
- DOI/PMID 在线动作、缺失标识和 host allowlist；
- v1/v2 traversal、缓存超前、缓存丢失/损坏、内容提交后控制失败、路径拒绝和磁盘容量错误；
- traversal checkpoint 重放不复制内容、重复生成相同变更事件或改变 ID；
- 受管直连、显式代理无直连 fallback，以及多进程代理秘密边界。
