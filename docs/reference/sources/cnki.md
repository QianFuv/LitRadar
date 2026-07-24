# CNKI 与浙江图书馆 Provider

CNKI 元数据索引、CNKI 在线摘要页和浙江图书馆全文是独立运行时边界。它们共享[规范文章契约](../index-provider-contract.md)，不通过索引 provenance 或持久 URL 互相绑定。

当前有两套 CNKI 元数据实现：

| 运行时名称 | 角色 | 主机/平台 | 能力 |
| --- | --- | --- | --- |
| `cnki` | 国内 NZKPT（默认中文索引） | `navi.cnki.net` / `kns.cnki.net`，`uniplatform=NZKPT` | `index_content` + `article_abstract` |
| `cnki_oversea` | 海外 CNKI | `oversea.cnki.net` | `index_content` + `article_abstract` |
| `zjlib` | 浙江图书馆全文 | 用户会话 | `article_full_text` only |

国内与海外页面和内部接口都不是 LitRadar 控制的稳定公共 API。上游页面变化应通过 fixture 和 parser 测试确认，不能通过在内容库新增 transport 字段规避。

## 能力声明

| 注册/实现 | 能力 | 进程与凭据 |
| --- | --- | --- |
| `cnki_index_registration` / live domestic transport | `IndexContentProvider` | `index` 父进程加载 `cnki_captcha_token`；多进程时只通过 stdin bootstrap 交给国内 worker |
| `cnki_access_registration` / API live domestic adapter | `ArticleAbstractProvider` | `serve` API 每次在线精确定位；读取同一加密 runtime secret |
| `cnki_oversea_index_registration` | `IndexContentProvider` | `index` 进程直连 CNKI overseas；不使用用户会话 |
| `cnki_oversea_access_registration` / API live adapter | `ArticleAbstractProvider` | `serve` API 每次在线精确定位；不使用 ZJLib 会话 |
| `zjlib` API registration | `ArticleFullTextProvider` | `serve` API 只读取当前用户已有的 active ZJLib CNKI 会话 |

逻辑名称 `cnki` 与 `cnki_oversea` 在索引与 API 进程分别注册实现，管理端把它们聚合为 `index_content + article_abstract`。国内 `cnki` **没有** fulltext。`zjlib` 是唯一内置全文 Provider。默认 `chinese_journals` 路由到 `cnki`；摘要默认 `scholarly → cnki`；全文默认 `zjlib`。

## 国内 NZKPT 索引流程

Provider 接收 LitRadar 维护的 `JournalCatalogEntry`，使用 canonical title、全部标题别名、印刷/电子 ISSN 和 `all_issns` 定位期刊：

1. 按规范顺序搜索全部维护标题，再搜索全部合法 ISSN（HAR 形 `searchStateJson`）。
2. 每个详情 URL 最多读取一次；候选与维护目录都提供 ISSN 且没有交集时，即使标题相同也拒绝。
3. ISSN 相交时可以接受 Provider 标题变体；任一侧没有 ISSN 时才回退到 canonical/alias 标题比较。
4. 从匹配详情页读取 `pykm`、`pCode` 和 yearList。
5. 按稳定 `year_issue_id` 和零基 `pageIdx` 读取 papers；每个 Provider batch 只处理一个经过计数验证的页面。
6. 页面内文章详情全部成功或仅有明确永久缺失后，映射为 `JournalDraft`、`IssueDraft`、`ArticleDraft`；所有 transport handle 和 URL 都在边界丢弃。
7. checkpoint 指向下一页或下一稳定期次；它不保存期次/文章数组下标，也不包含 captcha 字段。

同一索引进程处理一本期刊期间，首次 batch 取得的期刊详情和刊期树作为内存快照复用于后续页面；该刊完成后立即释放。新进程或新一轮已完成期刊索引会重新获取快照，因此 checkpoint 仍只依赖稳定 `year_issue_id`，不持久化上游句柄。

基础站点为 `https://navi.cnki.net` 与 `https://kns.cnki.net`。Transport 对初始 URL、Referer、challenge、每个 redirect hop 和最终 URL 使用同一规则：只允许这两个精确主机的 HTTPS 默认端口，拒绝 userinfo、IP literal、自定义端口、协议降级和跨域跳转。当前私有请求路径包括 journal 搜索、详情、year list、papers、article abstract 和 captcha verify API；这些路径不是内容契约。

### Captcha

国内请求可能返回 `blockPuzzle` 验证。LitRadar 使用 jfbym 通用双图滑块（type `20111`）：

1. 检测 `-403` / `/verify/home` / 安全验证正文；
2. `verify-api/get` 取背景与滑块图、`secretKey`；
3. 固定 HTTPS、禁止重定向的 jfbym dual-image 请求识别 gap x；只接受成功响应的 `data.data` 数字/纯数字字符串和 `0..=10_000` 的有限坐标；
4. AES-128-ECB PKCS7 加密 `pointJson` 后 `verify-api/web/check`；
5. 内存保留 `captchaId`，重试原请求；即使 challenge 出现在最后一次普通尝试，也会执行一次受预算限制的已认证重放。

密钥通过加密 runtime secret `cnki_captcha_token` 配置；数据库值为空时，单次索引探测可用 `LITRADAR_CNKI_CAPTCHA_TOKEN`。父进程解析该值后会从 child 环境移除变量；worker request JSON 不含 token，只有 `provider_name=cnki` 的 worker 在构造 Provider 前通过版本化 stdin bootstrap 收到它，后续同一管道继续传输 durable ACK。token、secretKey、captchaId 与图片不得进入 request 文件、日志、Debug、内容库或控制库。

## 海外 CNKI 索引流程

海外实现保持 `oversea.cnki.net` 流程：题名优先、ISSN fallback、year tree、articles、详情映射；checkpoint 语义与既有 adapter 一致。海外路径不使用 jfbym captcha secret。

## 规范字段映射

| 规范字段 | CNKI 页面来源/规则 |
| --- | --- |
| journal observation | 详情页标题、别名和 ISSN，仅用于验证维护目录项 |
| issue | 年期树的年份、卷、期、显示标题和日期 |
| `title` | 文章列表/详情规范文本 |
| `authors` | 只保留有序 display name |
| `abstract_text` | 详情页摘要文本 |
| `publication_year` / `date` | 年期和在线公开日期 |
| volume/issue/pages | 年期树、列表和详情页 |
| `doi` | 规范为小写 DOI 标识符，不保存 URL |
| `open_access` | 未知；列表的“免费/Free”不等同于规范 OA 结论 |

CNKI filename、`pykm`、`pCode`、数据库代码、详情路径、search URL、Cookie、captcha 和原始 HTML 只存在于私有 client/adapter 内。内容库没有 `platform_id`、`content_location`、`permalink` 或 `full_text_file`。

## Provider checkpoint

索引 adapter 可以把分页/年期进度编码为 opaque checkpoint。LitRadar 只把该文本保存在 `data/index-control/<catalog>.sqlite` 的 Provider namespace，并在下一次 `fetch` 原样传回。

国内 checkpoint 是指向下一处理位置的版本化 JSON，例如：

```json
{"version":1,"year_issue_id":"202512","page_index":1}
```

`year_issue_id` 必须精确匹配重新读取的 year list；新期次插到列表前部不会改变恢复目标。保存的期次已经消失时，Provider fail closed 并提示删除对应的可丢弃控制库后重扫，不按旧序号猜测位置。旧 `issue_index` / `article_index` 和任何 captcha 字段都会被拒绝。

papers 页必须含 `articleCount`，其值必须等于解析出的文章行数。计数为 10 时 checkpoint 指向同一期下一页；其他结构完整的计数进入下一稳定期次或完成，因为 CNKI 的历史期次可能在单页返回超过 10 条文章。总数正好为 10 的倍数时，必须再读取一个结构有效的空终止页；后续页的 `该刊数据正在更新中，请耐心等待` 是已确认的越界终止占位响应，可按 0-count 处理，但首个 papers 页出现同一响应仍然失败。空白、登录页、缺 marker 或局部行页面都是失败，不推进 checkpoint。

国内 CNKI Provider 读取详情页后，合并详情与 papers 行的作者字段，并规范化详情 DOI。仅当作者仍为空且 DOI 也为空时才排除该记录；标题和栏目不参与内容类型判断，因此带 DOI 的征稿启事以及有作者的书评会被保留。被排除的行不生成 `ArticleDraft`，但页面计数与 checkpoint 仍按原始响应推进。已有内容库不会因规则变更自动恢复之前排除的记录，应用新规则时应删除对应内容库和控制库再完整重建。

页面是最小提交和重放单元。只有 HTTP 404/410 或明确“记录已删除/文献不存在”的详情页会记录不含 URL/凭据的 ordinal/status 事件并跳过；网络错误、429、5xx、captcha 或结构错误会中止整个 batch，不返回新 checkpoint。控制库删除或更换 Provider 后从头读取，内容 writer 依靠规范 identity alias 幂等复用已有 ID。Provider 不能把 checkpoint 嵌入 `ArticleDraft`。

## 在线摘要页

摘要动作不会读取持久链接。每次请求都使用 `ArticleLocator` 的维护期刊题名/ISSN、文章题名、年份、卷期、页码、作者和 DOI 执行在线定位：

1. 精确定位期刊；
2. 读取相关年期的全部经过验证的 papers 页面和文章候选；
3. 打开候选详情并核对规范文章身份；
4. 只把本次匹配的 HTTPS 目的地返回给 API。

国内 allowlist：`navi.cnki.net`、`kns.cnki.net`、`www.cnki.net`。  
海外 allowlist：`oversea.cnki.net`、`kns.cnki.net`、`www.cnki.net`。

API 会再次执行统一 HTTPS/host 校验，返回 `Cache-Control: private, no-store` 的 307；目的地不写入内容、控制或认证库。

## 浙江图书馆全文

ZJLib 全文能力与 CNKI 索引 Provider 无关：

1. 用户在设置页完成浙江图书馆扫码登录；会话密文按用户保存在 `data/auth.sqlite` 的 CNKI session 表。
2. `/access` 只检查本地 active 状态。若后续还有无需登录的全文 Provider，ZJLib 未登录不会阻断回退按钮。
3. 用户调用 `/fulltext` 后，Provider 读取当前用户已有的 session snapshot。
4. 客户端完成 BFF/Share SSO、Cookie 同步和代理预热，然后按文章题名搜索。
5. 下载前规范化比较候选题名、作者和期刊；三项不匹配就拒绝 PDF。
6. 匹配 PDF 必须非空、`application/pdf` 且不超过 32 MiB，随后以 no-store attachment 返回。

HTTP 会话路径可能仍使用历史 `/api/cnki/*` 前缀，但 runtime Provider 名称是 `zjlib`。全文动作不会把更新后的 client Cookie 写回 session，不更新 `updated_at`/`last_used_at`，也不缓存 PDF 或新增文件。API 不返回 token、Cookie、代理 URL 或 transport 错误详情。

### 代理重定向安全

ZJLib 客户端手动处理已知的登录/代理主机跳转，只允许 HTTPS、允许主机、有限跳数和有效 `vpn358_sid` 成功门槛。已知双节点循环会有限重取登录地址；其他协议、主机、Location、循环或跳数异常明确失败。

reqwest 错误在转换为业务错误前移除完整 URL。需要诊断的自定义地址也必须脱敏查询参数，避免 `enc`、用户标识或 Cookie 信息进入日志/API。

## 重试和可观测性

单个 CNKI HTTP 操作最多有三次普通响应尝试。没有收到 HTTP 响应的传输失败最多尝试五次，并按 1、2、4、8 秒进行有界指数退避；国内 captcha 另有最多五次 fresh solve/replay 的独立预算。持续失败使当前 Provider 操作明确失败，不写空内容冒充成功；只有上述窄范围永久文章缺失可以在同页继续。

请求尝试只汇总到结构化 `index.provider.attempts` 或文章访问 fallback 事件。内容库没有 API/path statistics 表，也不保存 URL、响应正文、查询参数或解码器样本。

`--workers` 控制每个期刊子进程内当前 papers 页的并发详情请求，`--processes` 控制同一目录的 journal worker；期刊定位、刊期遍历、页面归并、checkpoint 和 SQLite 写入仍保持有序。默认值和内存边界见[CLI 参考](../cli.md)。

## 维护测试

修改 CNKI/ZJLib adapter 时至少覆盖：

- 题名优先、ISSN fallback 和候选期刊验证（国内与海外）；
- `pykm`/`pCode`、年期树、10+2 与 10+0 papers 分页、计数不一致和详情变体；
- captcha/验证页、非 2xx 和 decode retry；预算与 secret 脱敏；
- 稳定 `year_issue_id + page_index` 恢复、期次重排/消失、永久详情跳过和临时错误整页重放；
- batch 中没有 filename、Provider、URL 或原始 HTML；
- 在线摘要每次重新解析全部页面且 host 受限；
- ZJLib 用户隔离、题名/作者/期刊三项精确匹配和 32 MiB 上限；
- 成功、无匹配和 fallback 后索引/control/auth 行与文件系统均不变；
- zyproxy 协议、主机、跳数、循环和 URL 脱敏。
