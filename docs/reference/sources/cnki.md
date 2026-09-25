# CNKI 与浙江图书馆 Provider

CNKI 元数据索引、CNKI 在线摘要页和浙江图书馆全文是独立运行时边界。它们共享[规范文章契约](../index-provider-contract.md)，不通过索引 provenance 或持久 URL 互相绑定。

CNKI 元数据只使用国内 NZKPT 实现：

| 运行时名称 | 角色                       | 主机/平台                                             | 能力                                 |
| ---------- | -------------------------- | ----------------------------------------------------- | ------------------------------------ |
| `cnki`     | 国内 NZKPT（默认中文索引） | `navi.cnki.net` / `kns.cnki.net`，`uniplatform=NZKPT` | `index_content` + `article_abstract` |
| `zjlib`    | 浙江图书馆全文             | 用户会话                                              | 仅 `article_full_text`               |

国内页面和内部接口都不是 LitRadar 控制的稳定公共 API。上游页面变化应通过 fixture 和 parser 测试确认，不能通过在内容库新增 transport 字段规避。

## 能力声明

| 注册/实现                                              | 能力                      | 进程与凭据                                                                              |
| ------------------------------------------------------ | ------------------------- | --------------------------------------------------------------------------------------- |
| `cnki_index_registration` / live domestic transport    | `IndexContentProvider`    | `index` 父进程加载 `cnki_captcha_token`；多进程时只通过 stdin bootstrap 交给国内 worker |
| `cnki_access_registration` / API live domestic adapter | `ArticleAbstractProvider` | `serve` API 每次在线精确定位；读取同一加密 runtime secret                               |
| `zjlib` API registration                               | `ArticleFullTextProvider` | `serve` API 只读取当前用户已有的 active ZJLib CNKI 会话                                 |

`cnki` 注册国内索引与摘要页访问，管理能力目录报告 `index_content + article_abstract`。 国内 `cnki` **没有** fulltext。`zjlib` 是唯一内置全文 Provider。默认 `chinese_journals` 路由到 `cnki`；摘要默认 `scholarly → cnki`；全文默认 `zjlib`。

## 托管代理归属

[运行配置](../configuration.md)中的一个共用 `provider_proxy_url` 通过以下两个逻辑开关覆盖本文的全部出站 HTTP：

- `cnki`：国内索引、在线摘要定位、challenge/verify 流程，以及 JFBYM 双图识别请求。JFBYM 是国内 CNKI 的 captcha 子流程，没有独立 policy key。
- `zjlib`：扫码开始、扫码状态轮询、会话预热、BFF/Share SSO、搜索、候选验证和 PDF 下载；重定向与禁止重定向的两个 client 使用同一个决定。

每个开关缺省为关闭。关闭时客户端明确忽略系统代理变量并直连；打开时对应流程只走显式代理，代理不可达不会静默直连。保存代理设置不会热加载：`serve` 必须重启，索引由下一条新命令读取。代理 URL 和凭据不会进入 API 响应、CNKI session、内容/控制库、worker request JSON、日志或 Debug。

## 国内 NZKPT 索引流程

Provider 接收 LitRadar 维护的 `JournalCatalogEntry`，使用 canonical title、全部标题别名、印刷/电子 ISSN 和 `all_issns` 定位期刊：

1. 按规范顺序搜索全部维护标题，再搜索全部合法 ISSN（HAR 形 `searchStateJson`）。
2. 每个详情 URL 最多读取一次；候选与维护目录都提供 ISSN 且没有交集时，即使标题相同也拒绝。
3. ISSN 相交时可以接受 Provider 标题变体；任一侧没有 ISSN 时才回退到 canonical/alias 标题比较。
4. 从匹配详情页读取 `pykm`、`pCode` 和 yearList。
5. 按稳定 `year_issue_id` 和零基 `pageIdx` 读取 papers；每个 Provider batch 只处理一个经过计数验证的页面。
6. 页面内文章详情全部成功或仅有明确永久缺失后，映射为 `JournalDraft`、`IssueDraft`、`ArticleDraft`；所有 transport handle 和 URL 都在边界丢弃。
7. traversal checkpoint 保存冻结的 base/head、下一稳定期次和页码；它不保存期次/文章数组下标，也不包含 captcha 字段。

同一索引进程处理一本期刊期间，首次 batch 取得的期刊详情和刊期树作为内存快照复用于后续页面；该刊完成后立即释放。新进程或新一轮已完成期刊索引会重新获取快照，因此 checkpoint 仍只依赖稳定 `year_issue_id`，不持久化上游句柄。

期刊定位保留维护标题、别名、括号变体和 ISSN 的查询顺序。每次搜索后按顺序验证新发现的详情 URL，沿用身份与 ISSN 冲突检查，首个验证成功的匹配会停止后续搜索；详情 URL 在全部查询间去重。只有完整查询一轮仍没有候选 URL 时，才初始化一次 HTTPS `/knavi/` 并重试有序查询。已有但不匹配的候选不会触发导航恢复，必需请求或解析错误直接传播。

Incremental 从远端当前最新 `year_issue_id` 向旧扫描到 committed anchor，并完整包含 anchor 期次的全部 papers 页。首次确认的远端头部成为本次冻结 candidate；运行期间新增的更高期次留给下一次 update。只有闭区间全部完成后才返回 candidate 作为新 anchor。committed issue 已从 year list 消失时安全完整扫描；恢复中的 candidate/current 消失则 fail closed。FullRescan 忽略 anchor 停止边界并覆盖完整期次树。

基础站点为 `https://navi.cnki.net` 与 `https://kns.cnki.net`。Transport 对初始 URL、Referer、challenge、每个 redirect hop 和最终 URL 使用同一规则：只允许这两个精确主机的 HTTPS 默认端口，拒绝 userinfo、IP literal、自定义端口、协议降级和跨域跳转。当前私有请求路径包括 journal 搜索、详情、year list、papers、article abstract 和 captcha verify API；这些路径不是内容契约。

<a id="captcha"></a>

### 验证码处理

国内请求可能返回 `blockPuzzle` 验证。LitRadar 使用 jfbym 通用双图滑块（type `20111`）：

1. 检测 `-403` / `/verify/home` / 安全验证正文；
2. `verify-api/get` 取背景与滑块图、`secretKey`；
3. 固定 HTTPS、禁止重定向的 jfbym dual-image 请求识别 gap x；只接受成功响应的 `data.data` 数字/纯数字字符串和 `0..=10_000` 的有限坐标；
4. AES-128-ECB PKCS7 加密 `pointJson` 后 `verify-api/web/check`；
5. 内存保留 `captchaId`，重试原请求；即使 challenge 出现在最后一次普通尝试，也会执行一次受预算限制的已认证重放。

密钥通过加密 runtime secret `cnki_captcha_token` 配置；数据库值为空时，单次索引探测可用 `LITRADAR_CNKI_CAPTCHA_TOKEN`。父进程解析该值后会从 child 环境移除变量；worker request JSON 不含 token 或代理 URL，只有 `provider_name=cnki` 的 worker 在构造 Provider 前通过版本化 stdin bootstrap 收到 captcha token，只有当前 Provider 开关启用的 worker 才在同一 bootstrap 收到代理 URL。后续同一管道继续传输 durable ACK。token、代理 URL、secretKey、captchaId 与图片不得进入 request 文件、参数、环境、日志、Debug、内容库或控制库。

<a id="retired-overseas-provider"></a>

## 已退役的海外 Provider

海外运行时实现已移除。认证库 v19 把已保存的 `cnki_oversea` 选择迁移为 `cnki`，保留顺序、显式空覆盖和已有国内代理选择。控制库 v5 丢弃海外租约、成功边界和检查点，不能通过国内传输恢复这些状态；历史批次账本保持不变。

## 规范字段映射

| 规范字段                    | CNKI 页面来源/规则                            |
| --------------------------- | --------------------------------------------- |
| journal observation         | 详情页标题、别名和 ISSN，仅用于验证维护目录项 |
| issue                       | 年期树的年份、卷、期、显示标题和日期          |
| `title`                     | 文章列表/详情规范文本                         |
| `authors`                   | 只保留有序 display name                       |
| `abstract_text`             | 详情页摘要文本                                |
| `publication_year` / `date` | 年期和在线公开日期                            |
| volume/issue/pages          | 年期树、列表和详情页                          |
| `doi`                       | 规范为小写 DOI 标识符，不保存 URL             |
| `open_access`               | 未知；列表的“免费/Free”不等同于规范 OA 结论   |

CNKI filename、`pykm`、`pCode`、数据库代码、详情路径、search URL、Cookie、captcha 和原始 HTML 只存在于私有 client/adapter 内。内容库没有 `platform_id`、`content_location`、`permalink` 或 `full_text_file`。

## Provider anchor 与 checkpoint

索引 adapter 可以把成功边界和分页/年期进度分别编码为 opaque anchor 与 traversal checkpoint。LitRadar 只把文本保存在 `data/index-control/<catalog>.sqlite` 的 Provider namespace，并在下一次 `fetch` 原样传回；核心不解析 `year_issue_id`。

国内成功 anchor v1 只保存已经完整覆盖的最新稳定期次：

```json
{ "version": 1, "year_issue_id": "202602" }
```

国内 traversal checkpoint v2 指向同一冻结窗口中的下一处理位置，例如：

```json
{
  "version": 2,
  "base_anchor_issue_id": "202512",
  "candidate_head_issue_id": "202602",
  "current_issue_id": "202601",
  "page_index": 0
}
```

`base_anchor_issue_id` 是运行开始时冻结的成功边界，`candidate_head_issue_id` 是本次可推进的新边界，`current_issue_id/page_index` 是下一页。恢复时这些稳定 ID 必须精确匹配重新读取的 year list；新期次插到 candidate 前部会被忽略，不改变本次窗口。恢复中的 candidate/current 消失时 Provider fail closed，不按旧序号猜测位置。旧 v1、`issue_index` / `article_index` 和任何 captcha 字段都会被拒绝。

papers 页必须含 `articleCount`，其值必须等于解析出的文章行数。计数为 10 时 checkpoint 指向同一期下一页；其他结构完整的计数进入下一稳定期次或完成，因为 CNKI 的历史期次可能在单页返回超过 10 条文章。总数正好为 10 的倍数时，必须再读取一个结构有效的空终止页；后续页的 `该刊数据正在更新中，请耐心等待` 是已确认的越界终止占位响应，可按 0-count 处理，但首个 papers 页出现同一响应仍然失败。空白、登录页、缺 marker 或局部行页面都是失败，不推进 checkpoint。

国内 CNKI Provider 读取详情页后，合并详情与 papers 行的作者字段，并规范化详情 DOI。仅当作者仍为空且 DOI 也为空时才排除该记录；标题和栏目不参与内容类型判断，因此带 DOI 的征稿启事以及有作者的书评会被保留。被排除的行不生成 `ArticleDraft`，但页面计数与 checkpoint 仍按原始响应推进。已有内容库不会因规则变更自动恢复之前排除的记录，应用新规则时应删除对应内容库和控制库再完整重建。

页面是最小提交和重放单元。边界期次即使跨页或文章数正好是 10 的倍数，也必须读完其有效空终止页后才能 Complete。只有 HTTP 404/410 或明确“记录已删除/文献不存在”的详情页会记录不含 URL/凭据的 ordinal/status 事件并跳过；网络错误、429、5xx、captcha 或结构错误会中止整个 batch，不返回新 checkpoint。控制库删除或更换 Provider 后没有可信 anchor，会从头读取；内容 writer 依靠规范 identity alias 幂等复用已有 ID。Provider 不能把 anchor/checkpoint 嵌入 `ArticleDraft`。

可重试的国内页面失败保留最多 3 次尝试，以及会话和连接池重置。每次重试都重新获取并验证 papers 页面；只在同一次 `fetch` 调用中，目录、期次和完整有序页面都不变时，才复用成功转换的记录及有效过滤结果。任意行增删、重排、替换，或 URL、令牌、元数据变化都会清空整页缓存。在传播第一个页面错误前，先收集全部已完成的成功详情，包括序号在错误之后的成功项。

请求或转换错误、永久 404/410 缺失不缓存，缓存命中不增加 `SourceAttempt`。分页始终由新页面计数决定，最终失败不返回部分批次或新检查点。调用返回即销毁缓存，其他调用、页面、更新或重启仍正常刷新详情；期刊和年期树的较长生命周期快照不变。例如，同一双文章页面首次有一个详情失败时，papers 仍请求两次，详情调用可由 4 次降至 3 次。

## 在线摘要页

摘要动作不会读取持久链接。每次请求都使用 `ArticleLocator` 的维护期刊题名/ISSN、文章题名、年份、卷期、页码、作者和 DOI 执行在线定位：

1. 精确定位期刊；
2. 读取相关年期的全部经过验证的 papers 页面和文章候选；
3. 打开候选详情并核对规范文章身份；
4. 只把本次匹配的 HTTPS 目的地返回给 API。

国内 allowlist：`navi.cnki.net`、`kns.cnki.net`、`www.cnki.net`。

API 会再次执行统一 HTTPS/host 校验，返回 `Cache-Control: private, no-store` 的 307；目的地不写入内容、控制或认证库。

## 浙江图书馆全文

ZJLib 全文能力与 CNKI 索引 Provider 无关：

1. 用户在设置页完成浙江图书馆扫码登录；会话密文按用户保存在 `data/auth.sqlite` 的 CNKI session 表。
2. `/access` 只检查本地 active 状态。若后续还有无需登录的全文 Provider，ZJLib 未登录不会阻断回退按钮。
3. 用户调用 `/fulltext` 后，Provider 读取当前用户已有的 session snapshot。
4. 客户端完成 BFF/Share SSO、Cookie 同步和代理预热，然后按文章题名搜索。
5. 下载前规范化比较候选题名、作者和期刊；三项不匹配就拒绝 PDF。
6. 匹配 PDF 必须非空、`application/pdf` 且不超过 32 MiB，随后以 no-store attachment 返回。

HTTP 会话路径可能仍使用历史 `/api/cnki/*` 前缀，但 runtime Provider 名称是 `zjlib`。全文动作不会把更新后的 client Cookie 写回 session，不更新 `updated_at`/`last_used_at`，也不缓存 PDF 或新增文件。API 不返回 token、Cookie、代理 URL 或 transport 错误详情；start、timeout、login 和 warm-up 失败只返回固定 code/phase/message。上游 `success=false` JSON 中的 `desc`/`message` 会在 source 边界丢弃，即使该上游刚收到 BFF token，也不能借错误响应把自由文本带回客户端或日志。

登录 start 在网络调用前预留单调递增的 session generation，并在同一原子语句中清空旧 QR UUID；poll 绑定读取时的 generation 与 QR UUID。只有仍匹配的请求才能提交网络结果。新的 start 或 DELETE clear 会使所有更早的 start/poll 完成失效；等待新 start 网络结果时仍保留既有 active 会话材料，但旧 QR 已无法进入 poll。clear 保存加密空 tombstone 以保留该栅栏，同时对读取接口表现为未配置。陈旧完成固定返回 HTTP 409，错误码为 `cnki_login_superseded`，且不会恢复或覆盖凭据。

### ZJLib 上游代理跳转安全

这里的“代理主机”是浙江图书馆上游 zyproxy 跳转，不是 `provider_proxy_url` 的出站网络代理。ZJLib 客户端手动处理已知的登录/zyproxy 主机跳转，只允许 HTTPS、允许主机、有限跳数和有效 `vpn358_sid` 成功门槛。已知双节点循环会有限重取登录地址；其他协议、主机、Location、循环或跳数异常明确失败。启用 `zjlib` 托管代理不会放宽这些 host、scheme、Cookie 或跳数检查。

所有 ZJLib 请求还必须属于固定的 `www`、`share`、`zyproxy-login` 或 `zyproxy` endpoint family。实际请求只接受配置内的 HTTPS scheme、精确 host/default port 和路径边界；userinfo、fragment、编码后的路径分隔符或 dot-segment 均被拒绝。HTTP loopback 只存在于编译期测试 fixture，不属于实际连接配置。

Share 页面返回的 `domainUrl`、`portalContextPath`，CNKI 结果页返回的 absolute detail/download URL，以及每个自动重定向 Location 都会在发送前重新验证。自动重定向不得跨 family，并保留最多十跳的上限；因此上游响应不能把表单签名、Cookie 或下载请求转发到其他 origin。

reqwest 错误在转换为业务错误前移除完整 URL。需要诊断的响应地址会移除全部 query 与 fragment，避免 `enc`、用户标识、文章信息或 Cookie 内容进入日志/API。

## 重试和可观测性

国内 CNKI 的普通响应最多尝试 5 次，无响应传输最多尝试 8 次，使用有界指数退避；验证码识别与重放保留 5 次尝试预算。每个逻辑元数据请求使用一个 180 秒截止时间，并受调用方更早截止时间约束，覆盖重试、验证码锁等待、冷却和验证码网络工作。持续失败不生成空内容或新检查点。

HTTP 429 在读取或分类验证码正文之前处理。`Retry-After` 接受整数秒和 HTTP 日期；克隆的国内客户端共享观察到的最长冷却，临时会话重置也不会清除。429 不触发新验证码重放或识别，challenge GET 和验证 POST 也先发布 429 冷却，再继续后续验证码工作。等待时释放冷却锁，发送前重新检查；剩余时间不足以容纳必需等待时立即失败，不提前重试。正文超大或不可读不会清除已观察的状态与响应头，其他大小和永久缺失规则仍适用，ZJLib 重试行为不变。

CNKI Domestic 和 ZJLib 的 HTML/JSON 响应解压后上限均为 2 MiB，JFBYM JSON 为 256 KiB。读取先检查可用的 Content-Length，再对透明解压后的流最多保留 `limit + 1` 字节；因此 chunked 响应和 gzip 高压缩比都不能绕过上限。超限是固定分类的不可重试无效响应，不保留正文。PDF 仍使用独立的 32 MiB 有界读取。

请求尝试只汇总到结构化 `index.provider.attempts` 或文章访问 fallback 事件。内容库没有 API/path statistics 表，也不保存 URL、响应正文、查询参数或解码器样本。

国内 CNKI 默认 6 个详情工作线程和 1 个期刊执行器。2026 年 9 月 17 日的一次有界线上传输检查分别在并发 1、2、4、6 下完成 2、4、8、12 个详情任务，共享会话准备后未出现额外 429 或验证码挑战。该历史结果只支持被测账号和网络下的默认值，不是普遍的 CNKI 配额，也不是本次编辑的新测试。显式数量仍为 `1..=32`，聚合容量最多 32；期刊解析、期次遍历、页面组装、检查点和 SQLite 写入保持有序。配置值与实际执行器报告见[CLI 参考](../cli.md)。详情任务重叠不等于物理 HTTP 重叠，辅助验证码和识别请求不计入 `SourceAttempt`。

## 维护测试

修改 CNKI/ZJLib adapter 时至少覆盖：

- 题名优先、ISSN fallback 和候选期刊验证（国内）；
- `pykm`/`pCode`、年期树、10+2 与 10+0 papers 分页、计数不一致和详情变体；
- captcha/验证页、非 2xx 和 decode retry；预算与 secret 脱敏；
- 稳定 `year_issue_id + page_index` 恢复、期次重排/消失、永久详情跳过和临时错误整页重放；
- batch 中没有 filename、Provider、URL 或原始 HTML；
- 在线摘要每次重新解析全部页面且 host 受限；
- ZJLib 用户隔离、题名/作者/期刊三项精确匹配和 32 MiB 上限；
- 成功、无匹配和 fallback 后索引/control/auth 行与文件系统均不变；
- zyproxy 协议、主机、跳数、循环和 URL 脱敏；
- `cnki`、`zjlib` 代理归属，受管直连，以及多进程秘密只走 stdin bootstrap。
