# CNKI 与浙江图书馆 Provider

CNKI 元数据索引、CNKI 详情/摘要页和浙江图书馆全文是三个独立运行时边界。它们共享[规范文章契约](../index-provider-contract.md)，不通过索引 provenance 或持久 URL 互相绑定。

CNKI overseas 页面和内部接口不是 LitRadar 控制的稳定公共 API。上游页面变化应通过 fixture 和 parser 测试确认，不能通过在内容库新增 transport 字段规避。

## 能力声明

| 注册/实现                                     | 能力                                               | 凭据                                                  |
| --------------------------------------------- | -------------------------------------------------- | ----------------------------------------------------- |
| `cnki_index_registration`                     | `IndexContentProvider`                             | 直连 CNKI overseas；不使用用户会话                    |
| `cnki_access_registration` / API live adapter | `ArticleDetailProvider`、`ArticleAbstractProvider` | 每次动作在线精确定位；不使用 ZJLib 会话               |
| `zjlib_cnki` API registration                 | `ArticleFullTextProvider`                          | 只读取当前 LitRadar 用户已有的 active ZJLib CNKI 会话 |

三个能力可以独立排序、启用或替换。中文目录以后切换到其他索引 Provider 时，CNKI 在线详情或 ZJLib 全文仍可继续作为运行时候选。

## 元数据索引流程

Provider 接收 LitRadar 维护的 `JournalCatalogEntry`，按题名优先、ISSN fallback 定位期刊：

1. 用维护标题搜索“刊名（曾用刊名）”。
2. 打开候选详情页，同时核对规范题名/别名和 ISSN。
3. 题名路径无可信匹配时，以维护 ISSN 精确搜索。
4. 从匹配详情页读取 `pykm`、`pCode` 和年期树。
5. 分页读取每期文章列表，并在当前 Provider 调用内打开文章详情。
6. 映射为 `JournalDraft`、`IssueDraft`、`ArticleDraft`，丢弃所有 transport handle 和 URL。

基础站点为 `https://oversea.cnki.net`。当前私有请求路径包括 journal 搜索、详情、year list、papers 和 article abstract 页面；这些路径不是内容契约。

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

CNKI filename、`pykm`、`pCode`、数据库代码、详情路径、search URL、Cookie 和原始 HTML 只存在于私有 client/adapter 内。内容库没有 `platform_id`、`content_location`、`permalink` 或 `full_text_file`。

## Provider checkpoint

CNKI 索引 adapter 可以把分页/年期进度编码为 opaque checkpoint。LitRadar 只把该文本保存在 `data/index-control/<catalog>.sqlite` 的 CNKI namespace，并在下一次 `fetch` 原样传回。

Provider 不能把 checkpoint 嵌入 `ArticleDraft`。控制库删除或更换 Provider 后从头读取，内容 writer 依靠规范 identity alias 幂等复用已有 ID。

## 在线详情和摘要页

详情或摘要动作不会读取持久链接。每次请求都使用 `ArticleLocator` 的维护期刊题名/ISSN、文章题名、年份、卷期、页码、作者和 DOI 执行在线定位：

1. 精确定位期刊；
2. 读取相关年期和文章候选；
3. 打开候选详情并核对规范文章身份；
4. 只把本次匹配的 CNKI HTTPS 目的地返回给 API。

注册 allowlist 为 `oversea.cnki.net`、`kns.cnki.net` 和 `www.cnki.net`。API 会再次执行统一 HTTPS/host 校验，返回 `Cache-Control: private, no-store` 的 307；目的地不写入内容、控制或认证库。

## 浙江图书馆全文

ZJLib 全文能力与 CNKI 索引 Provider 无关：

1. 用户在设置页完成浙江图书馆扫码登录；会话密文按用户保存在 `data/auth.sqlite.cnki_sessions`。
2. `/access` 只检查本地 active 状态。若后续还有无需登录的全文 Provider，ZJLib 未登录不会阻断回退按钮。
3. 用户调用 `/fulltext` 后，Provider 读取当前用户已有的 session snapshot。
4. 客户端完成 BFF/Share SSO、Cookie 同步和代理预热，然后按文章题名搜索。
5. 下载前规范化比较候选题名、作者和期刊；三项不匹配就拒绝 PDF。
6. 匹配 PDF 必须非空、`application/pdf` 且不超过 32 MiB，随后以 no-store attachment 返回。

全文动作不会把更新后的 client Cookie 写回 session，不更新 `updated_at`/`last_used_at`，也不缓存 PDF 或新增文件。API 不返回 token、Cookie、代理 URL 或 transport 错误详情。

### 代理重定向安全

ZJLib 客户端手动处理已知的登录/代理主机跳转，只允许 HTTPS、允许主机、有限跳数和有效 `vpn358_sid` 成功门槛。已知双节点循环会有限重取登录地址；其他协议、主机、Location、循环或跳数异常明确失败。

reqwest 错误在转换为业务错误前移除完整 URL。需要诊断的自定义地址也必须脱敏查询参数，避免 `enc`、用户标识或 Cookie 信息进入日志/API。

## 重试和可观测性

单个 CNKI HTTP 操作最多三次，两次等待分别为 1 秒和 2 秒。传输失败、非 2xx、验证码或异常验证页会重试；持续失败使当前 Provider 操作明确失败，不写空内容冒充成功。

请求尝试只汇总到结构化 `index.provider.attempts` 或文章访问 fallback 事件。内容库没有 API/path statistics 表，也不保存 URL、响应正文、查询参数或解码器样本。

当前没有代理池运行设置。`--workers` 和 `--issue-batch` 只控制 CNKI adapter 内的详情工作，`--processes` 控制同一目录的 journal worker；默认值和内存边界见[CLI 参考](../cli.md)。

## 维护测试

修改 CNKI/ZJLib adapter 时至少覆盖：

- 题名优先、ISSN fallback 和候选期刊验证；
- `pykm`/`pCode`、年期树、文章列表与详情变体；
- captcha/验证页、非 2xx 和 decode retry；
- batch 中没有 filename、Provider、URL 或原始 HTML；
- 在线详情/摘要每次重新解析且 host 受限；
- ZJLib 用户隔离、题名/作者/期刊三项精确匹配和 32 MiB 上限；
- 成功、无匹配和 fallback 后索引/control/auth 行与文件系统均不变；
- zyproxy 协议、主机、跳数、循环和 URL 脱敏。
