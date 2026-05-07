# CNKI overseas 接口解析说明

本文档说明中文期刊索引使用的 CNKI overseas 路线。当前中文期刊 CSV `source` 使用 `cnki`。

验证日期：2026-05-06。

## 一、实现定位

CNKI overseas 的期刊页不是纯静态目录页。索引器按以下流程解析：

1. 用 ISSN 或刊名调用期刊检索接口，拿到期刊详情页 URL。
2. 请求期刊详情页，从隐藏字段提取 `pykm`、`pCode` 和 `time`。
3. 用 `yearList` 接口获取所有年份与期次。
4. 用 `papers` 接口获取某一期的文章列表。
5. 请求文章 `abstract` 页，解析题名、作者、摘要、DOI、页码、在线公开日期和阅读链接。

示例页面：

| 类型 | URL |
| --- | --- |
| 期刊页 | `https://oversea.cnki.net/knavi/detail?...&uniplatform=OVERSEA&language=chs` |
| 文章页 | `https://oversea.cnki.net/kcms2/article/abstract?...&uniplatform=OVERSEA&language=CHS` |

## 二、期刊检索接口

端点：

- `POST https://oversea.cnki.net/knavi/journals/searchbaseinfo`

关键表单字段：

| 字段 | 说明 |
| --- | --- |
| `searchStateJson` | CNKI 检索状态 JSON |
| `displaymode` | 固定为 `1` |
| `pageindex` | 页码，当前使用 `1` |
| `pagecount` | 每页数量，当前使用 `21` |
| `searchType` | `ISSN` 或 `刊名(曾用刊名)` |
| `switchdata` | 固定为 `search` |

`searchStateJson` 中当前使用：

| 字段 | ISSN 检索 | 刊名检索 |
| --- | --- | --- |
| `CNode.PCode` | `BOJHD70J` | `BOJHD70J` |
| `QNode.QGroup[0].Items[0].Name` | `SN` | `TI` |
| `QNode.QGroup[0].Items[0].Operate` | `=` | `%` |
| `QNode.QGroup[0].Items[0].Value` | ISSN | 期刊名 |

响应是 HTML 片段。实现会提取 `/knavi/detail?...` 链接，再进入期刊详情页解析。

## 三、期刊详情页

详情页用于拿到后续接口必需参数：

| 字段 | 来源 |
| --- | --- |
| `pykm` | `<input id="pykm" ...>` |
| `pCode` | `<input id="pCode" ...>`，通常为 `CJFD,CCJD` |
| `time` | `<input id="time" ...>` |
| 期刊名 | `<input id="shareChName" ...>` 或 `<title>` |
| ISSN / CN / 影响因子 | 页面可见文本 |

`time` 是目录接口需要的页面 token，因此不能只凭 CSV 中的 `id` 拼接目录接口。

## 四、年份与期次接口

端点：

- `POST https://oversea.cnki.net/knavi/journals/{pykm}/yearList`

表单字段：

| 字段 | 值 |
| --- | --- |
| `pIdx` | `0` |
| `time` | 期刊详情页隐藏字段 |
| `isEpublish` | 空字符串 |
| `pcode` | 期刊详情页 `pCode` |

响应是 HTML 年期树。实现解析 `id="yqYYYYNN"` 和 `value="..."`：

| 解析字段 | 说明 |
| --- | --- |
| `year` | `yq` 后前四位 |
| `number` | `yq` 后剩余期号，去掉前导零 |
| `year_issue` | `value` 属性，传给文章列表接口 |
| `title` | 期次显示文本 |

## 五、文章列表接口

端点：

- `POST https://oversea.cnki.net/knavi/journals/{pykm}/papers`

Query 参数：

| 参数 | 值 |
| --- | --- |
| `yearIssue` | 年期树中的 `value` |
| `pageIdx` | `0` |
| `pcode` | 期刊详情页 `pCode` |
| `isEpublish` | 空字符串 |
| `language` | `CHS` |

响应是某一期的 HTML 文章列表。实现解析：

| 字段 | 来源 |
| --- | --- |
| `article_url` | `/kcms2/article/abstract?...` 链接，并强制 `language=CHS` |
| `platform_id` | `<b name="encrypt" id="...">`，通常是 CNKI filename |
| `title` | 文章链接文本 |
| `authors` | `<span class="author" title="...">` |
| `pages` | `<span class="company" title="...">` |
| `is_free` | 行内是否出现 `免费` / `Free` |

## 六、文章详情页

文章详情页解析字段：

| 索引字段 | CNKI 来源 |
| --- | --- |
| `platform_id` | `paramfilename` / `param-filename` |
| `title` | `<p class="title-one">` |
| `authors` | `id="authorpart"` 作者块 |
| `abstract` | `<input id="abstract_text" value="...">` |
| `doi` | `DOI：` 信息行 |
| `date` | `在线公开时间` 的日期部分 |
| `start_page` / `end_page` | `页码：` |
| `permalink` / `content_location` | `openlink/detail?dbcode=...&dbname=...&filename=...` |
| `full_text_file` | 页面存在的 `HTML阅读` 链接 |

`HTML阅读`、CAJ、PDF 等链接是 CNKI 权限控制入口。索引器只保存链接，不下载或保存全文二进制。

## 七、并发与风控

当前未接入代理池，也不读取任何代理配置。CNKI 客户端直连海外站。

已验证的直连请求结果：

| 测试 | 结果 |
| --- | --- |
| 20 篇文章详情，串行，0.2 秒间隔 | 20/20 成功，约 0.44 rps |
| 40 篇文章详情，并发 4 | 40/40 成功，约 1.81 rps |
| 44 个混合请求，并发 8 | 44/44 成功，约 3.51 rps |
| 60 篇文章详情，并发 20 | 60/60 成功，约 7.94 rps |
| 80 篇文章详情，并发 32 | 80/80 成功，约 13.00 rps |
| 120 篇文章详情，并发 64 | 120/120 成功，约 16.44 rps，但 p95 延迟升高 |

当前默认索引配置按这个结果收敛为：

| 配置 | 默认值 |
| --- | --- |
| `--workers` | `32` |
| `--processes` | `2` |
| `--issue-batch` | 默认等于 `workers` |
| CNKI 文章详情并发上限 | `32` |

实现会把 403、429、3xx 跳转、`captcha`、`访问异常`、`安全验证` 视为风控或不可用响应并立即失败，避免静默写入空字段。全量重建是当前最有效的持续压力测试。

`workers` 控制每个进程内同时进行的 HTTP 请求数。`processes` 控制同时处理多少本期刊，因此全局并发上限大致是 `workers * processes`。`issue-batch` 控制每轮合并多少个 issue 的文章列表和详情请求；CNKI 路线会把同一 batch 内多个 issue 的文章详情一起调度，用于填满 `workers`。
