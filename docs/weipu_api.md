# 维普（CQVIP）抓取说明

本文档说明仓库中 `scripts/weipu/client.py` 的真实实现方式，用于抓取维普期刊与文章元数据。

## 一、实现概览

当前客户端类为：

- `WeipuAPISelectolax`

其技术路径是：

- `httpx`：发起 HTTP 请求
- `selectolax`：解析 HTML
- `quickjs`：执行页面中的 Nuxt 数据脚本
- 自带 `DES-ECB` 实现：生成签名

重要更正：

- 旧文档或旧注释里提到的 “Node.js” 已不准确
- 当前代码实际使用的是 `quickjs`，不是 Node.js 运行时

## 二、核心 URL

| 类型 | URL |
| --- | --- |
| 站点首页 | `https://www.cqvip.com` |
| 新站 API 前缀 | `https://www.cqvip.com/newsite` |
| 期刊检索 | `https://www.cqvip.com/journal/search?...` |
| 期刊详情 | `https://www.cqvip.com/journal/{journal_id}/{journal_id}` |
| 期次页 | `https://www.cqvip.com/journal/{journal_id}/{issue_id}` |
| 文献详情 | `https://www.cqvip.com/doc/journal/{article_id}` |

## 三、为何不能直接把页面当 JSON 解析

CQVIP 页面使用 Nuxt 服务端渲染，关键数据通常被嵌入：

- `window.__NUXT__`

或类似的页面脚本里，而不是直接提供干净的 JSON 接口给公开页面调用。

因此客户端的流程是：

1. 请求 HTML 页面
2. 用 `selectolax` 找到承载 Nuxt 数据的脚本
3. 用 `quickjs` 执行脚本
4. 拿到 payload 后做字段归一化

## 四、状态提取与签名

客户端会从页面 payload 中提取并缓存：

- `uuid`
- `env`
- `serverTime`

这些信息会用于后续 API 请求签名与时间对齐。

### 1. 时间对齐

客户端会记录服务器时间与本地时间的偏移量，后续签名尽量使用与服务端一致的时间戳。

### 2. Header 签名

当前实现使用：

- HMAC-SHA1
- 原始数据格式：`{app_id}\n{secret}\n{timestamp_sec}`
- 输出：Base64 字符串

### 3. Body 签名

对某些 CQVIP 新站接口，客户端会计算基于 `DES-ECB` 的请求体签名。

对应实现：

- `scripts/weipu/des.py`

## 五、主要入口方法

以下方法是当前索引流程真正会用到的核心入口。

### 1. 期刊搜索

| 方法 | 作用 |
| --- | --- |
| `search_journal_by_issn(issn)` | 按 ISSN 搜索期刊 |
| `search_journal_by_title(title)` | 按标题搜索期刊 |

索引器在直接获取期刊详情失败时，会先尝试按 ISSN 搜索，再按标题搜索。
当搜索命中新的 CQVIP `journalId` 时，索引器会把该值写入 `journals.platform_journal_id`，
而内部 `journal_id` 仍由 CSV 原始 ID 稳定派生，避免既有数据库关系需要重建。

### 2. 期刊详情

| 方法 | 作用 |
| --- | --- |
| `get_journal_details(journal_id)` | 获取期刊详情、年份与期次信息 |

返回的数据会进一步被 `build_weipu_journal_record()` 与 `build_weipu_issue_record()` 转成统一数据库格式。

### 3. 期次文章

| 方法 | 作用 |
| --- | --- |
| `get_issue_articles(journal_id, issue_id)` | 获取某个期次下的文章列表 |

索引器会把返回 payload 中的 `articles` 列表转成 `articles` 表记录。

## 六、字段归一化

当前维普抓取并不把上游字段原样入库，而是通过 `scripts/weipu/parsers.py` 做清洗：

- 期刊基础字段
- ISSN 规范化
- 作者列表规范化
- DOI 规范化
- 页码区间拆分
- 关键词清洗
- 详情链接补全

数据库写入侧再由 `scripts/index/transforms.py` 统一转换为：

- `build_weipu_journal_record`
- `build_weipu_issue_record`
- `build_weipu_article_record`

## 七、与主索引流程的关系

当 CSV 行中的：

- `library = -1`

时，索引器会把该期刊视为维普期刊，并走维普抓取路径。

对应主流程位于：

- `scripts/index/fetcher.py`

维普路径与 BrowZine 路径最终都会写入同一套 SQLite 索引表结构，因此前端与 API 层不需要感知来源差异。

## 八、鲁棒性策略

当前客户端内置了基础重试与退避：

- 对网络失败重试
- 对 `429 / 500 / 502 / 503 / 504` 重试
- 使用指数退避等待

这也是维普站点在高延迟或临时限流下仍能继续索引的关键保障。

## 九、常见失效点

如果维普抓取突然大量失败，优先排查：

1. 页面结构是否变更，导致 Nuxt 脚本提取失败
2. 签名规则是否发生变化
3. `journalId` / `issueId` 页面路由是否调整
4. 某些字段是否从页面 payload 中移除或改名

## 十、开发建议

调试维普抓取时，优先检查：

- `get_journal_details()` 的原始 payload
- `get_issue_articles()` 返回的 `articles`
- `scripts/weipu/parsers.py` 中的字段选择逻辑

如果页面脚本结构变化，通常不需要重写整个索引器，只需要修正：

- HTML 脚本定位
- QuickJS 执行输入
- parser 归一化规则
