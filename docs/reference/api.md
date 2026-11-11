# API 参考

本文档补充 OpenAPI 不便表达的认证方式、跨接口约定与业务边界。字段、请求体和响应 schema 以运行中的 OpenAPI 为准：

- API 文档页：`/docs`（huma 内置的 Stoplight Elements 页面，脚本和样式从 unpkg.com 加载；`/docs/` 重定向到 `/docs`）
- OpenAPI JSON：`/openapi.json`（OpenAPI 3.1）
- 前端生成基线：`apps/client/frontend/lib/generated/openapi.json`

OpenAPI 由 Go 类型生成：`apps/server/internal/api/` 用 huma 注册各操作的请求与响应类型，`apps/server/internal/openapi/` 负责操作 ID、共享 schema 注册和文档配置。修改 REST 接口后，应在 `apps/client/frontend/` 运行 `pnpm generate:api` 重新生成前端基线；不要在本文重复维护完整 schema。

## 地址与认证

本地默认地址为 `http://localhost:8000`（开发时由 Vite 代理，部署时由 Caddy 网关同源转发）。Go 服务端提供 `/api`、`/health/*`、`/docs`、`/openapi.json`、`/openapi.yaml`（`/docs` 页面读取它）和 `/mcp`，REST 路径统一以 `/api` 开头；网页由独立的 Web 客户端提供，服务端根路径返回 404。支持两种认证方式：

| 使用场景                      | 凭据                                                  |
| ----------------------------- | ----------------------------------------------------- |
| 同源 Web 前端                 | 登录后由后端设置的 `litradar_session` HttpOnly Cookie |
| 脚本、API 客户端与 MCP 客户端 | `Authorization: Bearer <access_token>`                |

访问令牌由 `POST /api/auth/tokens` 创建。不得把会话或访问令牌放入 URL 查询参数。

以下健康接口位于 REST `/api` 前缀之外且无需认证：

- `GET /health/live`
- `GET /health/ready`

以下 REST 接口无需认证：

- `GET /api/announcements`
- `POST /api/auth/register`
- `POST /api/auth/login`
- `GET /api/auth/invite-required`

`/api/admin/*` 需要管理员身份，其余接口需要普通用户或管理员身份。

<a id="cfp-tracking"></a>

## 征稿追踪

`GET /api/cfp/journals?db=NAME&q=TEXT` 返回完整维护期刊目录，包括尚无文章的期刊，以及后端计算的来源覆盖、数量和新鲜度，不含征稿正文。

`GET /api/cfp/journals/{catalog_id}/notices?db=NAME&include_closed=false&limit=50` 返回征稿原文、服务端计算的状态和首次投稿截止日期。`limit` 最大为 200；`catalog_id` 是不透明的维护目录身份或显式历史别名，`db` 是必填的元数据目录数据库文件名。有效但尚未适配的成员返回空的 `200` 响应，不存在的目录或成员返回 `404`。

下一页的 `cursor` 令牌通过认证加密绑定数据库、规范期刊、是否包含已关闭征稿、排序、元数据及来源版本和状态计算时刻，15 分钟后过期。篡改、跨查询、过期或数据版本已变化的令牌返回 `409`；客户端必须丢弃已累积的页面，不带游标重新开始。征稿字段使用 camelCase，既有 `page` 分页字段仍使用 snake_case。

征稿 GET 请求只读持久数据；采集、原文解析、Obscura/PDF 辅助程序和 SQLite 发布都由后端负责。操作入口见[征稿 CLI](cli.md#cfp)，采集和可用性边界见[征稿追踪架构](../architecture/cfp-tracking.md)。

## 通用约定

### 索引数据库选择

读取索引的接口接受可选 `db` 查询参数。值可以是 `data/index/` 下的 SQLite 文件名，也可以省略 `.sqlite` 后缀；路径部分会被丢弃。

- 只有一个索引库时可以省略 `db`。
- 存在多个索引库却未指定 `db` 时返回 `400`。
- 指定的索引库不存在时返回 `404`。

### ID 与分页

文章和期刊 ID 在 JSON 中序列化为十进制字符串，避免 JavaScript 丢失 64 位整数精度；路径参数和查询参数仍使用十进制文本。

列表接口通常采用 `limit` + `offset`。`GET /api/articles` 还支持游标分页：

- `cursor` 格式为 `{date}|{article_id}`；无日期记录使用空的 `{date}` 部分。
- 查询读取 `limit + 1` 条并移除哨兵行，因此结果恰好等于 `limit` 时 `has_more=false` 且没有 `next_cursor`。
- 未显式传 `include_total` 时，offset/首屏请求默认为 `true`，cursor 请求默认为 `false`；`false` 会跳过总数查询，此时 `page.total` 为 `null`。
- cursor 请求显式传 `include_total=true` 时，`total` 是不含 cursor/offset 条件的完整过滤结果总数。
- 精确的默认值、上限和过滤字段以 OpenAPI schema 为准。

全文查询 `q` 默认使用 `search_mode=simple`，把完整输入转义为一个 FTS5 字面短语，引号和 `OR` 等符号不会被解释为运算符。输入先按[检索分词](database.md#article_search)折叠重音和大小写并切词，因此 `resume` 能命中 `Résumé`，中文按字组成短语。只有显式设置 `search_mode=advanced` 才启用 FTS5 查询语法，其中每个操作数同样在 Go 中切成短语，`AND`/`OR`/`NOT`、`NEAR`、列限定和前缀 `*` 保持 FTS5 语义；非法高级表达式返回 `400 Invalid search expression`。REST、MCP 与 storage 均限制搜索文本最多 2,048 个 Unicode 字符、重复 `journal_id`/`area` 与期刊评级过滤值合计最多 500 项。

<a id="journal-rating-filters"></a>

### 期刊评级筛选

`GET /api/articles` 和 `GET /api/journals` 接受重复的 `utd_rating`、`abs_rating`、`fms_rating` 和 `fmscn_rating` 参数；MCP 的 `search_articles`、`list_journals` 接受同名数组。同一评级体系内使用 OR，不同体系及其他适用筛选条件之间使用 AND。例如，`abs_rating=4&abs_rating=4%2A&fms_rating=A` 表示 `(ABS 4 OR ABS 4*) AND FMS A`。`4*` 是精确等级，不是通配符或阈值。

评级值会去除首尾空白并去重。省略参数或传入空 MCP 数组表示不限制该体系，显式空白字符串则无效。未知的非空等级与冲突筛选返回空结果。所有输入项在去重前计入共享的 500 项上限，每个等级最多 2,048 个 Unicode 字符。评级同时约束页面成员与完整筛选总数；后续页必须保留筛选条件，条件变化时重置游标。

`GET /api/meta/ratings?db=NAME` 和 MCP `list_journal_ratings` 返回以上述参数命名的四组选项。选项结构例如 `{ "value": "4*", "count": 47 }`，其中数字仅用于说明格式，计数对象是所选数据库的期刊，包括无文章的期刊。无评级期刊不会出现在选项中，也不匹配已选等级；无评级数据的库返回四个空数组。选项直接读取期刊元数据，无需重建文章投影。首页评级不会隐式筛选独立的收藏、周报或征稿接口。

### 错误

错误响应使用 huma 默认的 RFC 9457 `application/problem+json` 格式：

```json
{
  "title": "Bad Request",
  "status": 400,
  "detail": "Readable error message",
  "errors": [{ "location": "body.field", "message": "Validation diagnostic" }]
}
```

模型包含 `type`、`title`、`status`、`detail`、`instance` 和 `errors`，未设置的可选成员会省略。响应不含顶层 `code` 或 `retryable`。可重试的 `429` 和 `503` 响应带 `Retry-After`（非负秒数），客户端据此决定是否重试。

huma 在调用业务 handler 之前解码并按 OpenAPI schema 校验 typed path、query、header 和 JSON 对象请求体，因此 schema 校验失败返回 `422`，并先于认证检查；JSON 语法错误返回 `400`。`errors[].location` 指出失败位置，例如 `body.field`、`query.limit` 和 `path.article_id`。handler 内部的业务校验仍返回 `400`。业务错误和原始 HTTP 边界共用 huma 的错误模型；`errors[]` 保留 `code`、`phase`、`action`、`request_id` 等分类位置，`Retry-After` 头按原有策略返回。

浙江图书馆 QR 登录轮询的 `interval_seconds` 为 `0.1..=10`（默认 2），`timeout_seconds` 为 `1..=600`（默认 180）；越界值在认证前返回 `422`。

分类的工作流错误把机器可读的值放进 `errors`：`location` 是分类名，`message` 是值；面向人的工作流消息放在 `detail`。

| 工作流                     | `errors[].location`  |
| -------------------------- | -------------------- |
| CNKI 登录、轮询和预热      | `code`、`phase`      |
| 文章 Provider 认证         | `code`、`action`     |
| 会话撤销未确认             | `code`、`request_id` |

例如 CNKI 登录超时包含 `{"location":"code","message":"cnki_login_timeout"}` 和 `{"location":"phase","message":"login"}`。

常见状态码：

| 状态码 | 含义                                                            |
| ------ | --------------------------------------------------------------- |
| `400`  | JSON 语法错误，或数据库选择、业务输入无效                       |
| `401`  | 缺少凭据、会话失效或 Bearer 格式错误                            |
| `403`  | 当前用户没有管理员权限                                          |
| `404`  | 数据库或记录不存在                                              |
| `409`  | 用户名、文件夹名等唯一约束冲突                                  |
| `422`  | 请求未通过 OpenAPI schema 校验（类型、必填、长度、范围、枚举）  |
| `428`  | 所有可用在线 Provider 都要求先完成认证                          |
| `429`  | 认证请求触发进程内限流；可重试并包含 `Retry-After`              |
| `503`  | 内嵌调度尚未 ready 或服务暂时不可用；可重试并包含 `Retry-After` |

服务端不会在通用 `500` / `503` 响应中暴露内部错误细节。

## REST 端点目录

本节只提供导航和职责边界。参数与响应字段请直接查看 `/docs` 或 `/openapi.json`。

### 健康、公告与索引读取

| 方法  | 路径                                  | 作用                             |
| ----- | ------------------------------------- | -------------------------------- |
| `GET` | `/health/live`                        | 进程 HTTP 存活状态               |
| `GET` | `/health/ready`                       | 最近 90 秒内是否存在内嵌调度心跳 |
| `GET` | `/api/announcements`                  | 当前启用的公告                   |
| `GET` | `/api/meta/databases`                 | 可用索引库                       |
| `GET` | `/api/meta/areas`                     | 领域与数量                       |
| `GET` | `/api/meta/ratings`                   | 期刊评级与期刊数量               |
| `GET` | `/api/meta/journals`                  | 期刊筛选选项                     |
| `GET` | `/api/years`                          | 出版年份汇总                     |
| `GET` | `/api/journals`                       | 期刊列表                         |
| `GET` | `/api/journals/{journal_id}`          | 单个期刊                         |
| `GET` | `/api/issues`                         | 期次列表                         |
| `GET` | `/api/issues/{issue_id}`              | 单个期次                         |
| `GET` | `/api/articles`                       | 文章过滤、全文检索与分页         |
| `GET` | `/api/articles/{article_id}`          | 单篇文章                         |
| `GET` | `/api/articles/{article_id}/access`   | 本地计算摘要页和全文能力         |
| `GET` | `/api/articles/{article_id}/abstract` | 在线解析并 307 跳转到摘要页      |
| `GET` | `/api/articles/{article_id}/fulltext` | 在线解析并返回 307 或有界 PDF    |
| `GET` | `/api/weekly-updates`                 | 按数据库和期刊聚合变更清单       |
| `GET` | `/api/weekly-updates/summary`         | 不含文章正文的周更分组计数       |
| `GET` | `/api/weekly-updates/articles`        | 固定周更窗口的文章游标页         |

周更接口读取 `data/push_state/*.changes.json` 及受管 history 中七日窗口内的可通知文章，不会临时重新抓取数据。兼容接口 `/api/weekly-updates` 在窗口内每个数据库去重后的文章引用合计不超过 2,000 条时保持原响应结构；第 2,001 条会在加载完整文章元数据前使请求返回 `413`，调用方应迁移到下述有界接口。

`/api/weekly-updates/summary` 只返回固定的 `window_start`、`window_end` 以及数据库/期刊标签和计数，不包含 article 数组或 abstract。`/api/weekly-updates/articles` 要求 `db`、正整数 `journal_id` 和摘要响应的 RFC3339 `window_end`，支持最长 2,048 字符的可选简单短语 `q`、`1..=200` 的 `limit`（默认 50）以及降序 `date|article_id` 游标。文章接口重新读取以该 `window_end` 截止的七日 manifest 成员，因此摘要之后生成的 manifest 不会混入后续页；成员 ID 仅写入当前 SQLite 连接的临时表，期刊、FTS、游标和 `limit + 1` 均在 SQL 内完成，不修改持久 schema 或索引内容。

期次、文章和 weekly article 的 `date` 只使用经真实日历校验的 `YYYY`、`YYYY-MM` 或 `YYYY-MM-DD`；配套 `date_precision` 为 `year`、`month`、`day` 或 `null`。只有年份的上游元数据保持如 `2026`，不会补成 `2026-01-01`。历史库中无法通过日历校验的旧日期仍原样可读，但其 precision 为 `null`。

健康响应的 `status` 只可能是 `ok` 或 `unhealthy`。应用自有 manual、scheduler 和 push-stat 状态同样由 OpenAPI enum 约束；服务不会返回任意拼写。CNKI 上游状态不属于该封闭集合，未知字符串会原样返回，客户端必须保留兼容分支。

### 认证与 CNKI 会话

| 方法                      | 路径                           | 作用                                 |
| ------------------------- | ------------------------------ | ------------------------------------ |
| `POST`                    | `/api/auth/register`           | 使用邀请码注册普通用户               |
| `POST`                    | `/api/auth/login`              | 登录并设置会话 Cookie                |
| `GET`                     | `/api/auth/invite-required`    | 注册与首管理员初始化状态             |
| `GET`                     | `/api/auth/me`                 | 当前用户                             |
| `POST`                    | `/api/auth/change-password`    | 修改当前用户密码                     |
| `POST`                    | `/api/auth/logout`             | 注销当前会话                         |
| `POST`                    | `/api/auth/logout-all`         | 撤销当前用户的全部会话与访问令牌     |
| `GET` / `POST`            | `/api/auth/tokens`             | 列出或创建访问令牌                   |
| `DELETE`                  | `/api/auth/tokens/{token_id}`  | 吊销访问令牌                         |
| `GET` / `POST` / `DELETE` | `/api/auth/invite-code`        | 查看、生成或永久撤销当前用户的邀请码 |
| `POST`                    | `/api/auth/invite-code/rotate` | 撤销旧邀请码并原子签发替代码         |
| `GET` / `DELETE`          | `/api/cnki/session`            | 查看或清除当前用户的 CNKI 会话       |
| `POST`                    | `/api/cnki/login/start`        | 启动浙江图书馆扫码登录               |
| `POST`                    | `/api/cnki/login/poll`         | 轮询扫码登录状态                     |

公开注册始终要求有效邀请码，且只能创建非管理员。首个管理员必须在本机通过 `litradar admin bootstrap` 创建，API 不提供远程引导端点。新密码至少为 12 个 Unicode 字符。

普通用户邀请码默认有效 7 天、最多注册 1 次。`GET` 返回 `status=active|expired|revoked|exhausted`、`expires_at`、`revoked_at`、`max_uses` 与 `use_count`；`rotate` 在一个事务中永久撤销当前未撤销码并创建替代码。过期、撤销或用尽的邀请码均不能注册，重复撤销返回 `404`。

浏览器登录 Cookie 使用固定 7 天有效期，每次登录轮换，不因普通 API 访问而滚动延长。登录写入会原子复核密码验证时观察到的令牌代际；若改密、管理员重置或 `logout-all` 已先提交，旧验证结果只会得到认证失败，不能创建撤销后的新 Cookie。`POST /api/auth/logout` 只撤销当前凭据；`POST /api/auth/logout-all` 在一个事务中递增令牌代际并撤销该用户的浏览器登录令牌和全部 Personal Access Token。两个端点对携带 `litradar_session` 的请求无论成功或失败都返回清除 Cookie 的 `Set-Cookie`；`logout` 返回 `401` 表示请求到达前令牌已失效，第一方浏览器将其视为幂等注销完成。SQLite busy/locked 只执行一次短时重试；若持久删除仍未确认，返回 `503`，响应示例如下：

```json
{
  "title": "Service Unavailable",
  "status": 503,
  "detail": "Session revocation could not be confirmed",
  "errors": [
    { "location": "code", "message": "session_revocation_unconfirmed" },
    { "location": "request_id", "message": "server-generated-request-id" }
  ]
}
```

浏览器此时必须清除非秘密本地用户快照，但不能声称服务端令牌已经撤销；第一方界面会保留跨刷新的警告和 request ID，并要求重新输入账号密码。重新认证取得一个新 Cookie 后，界面立即调用 `/api/auth/logout-all`，而不是尝试重放已清除的旧 Cookie。只有该请求成功后才清除警告。

登录页的 `next` 查询参数只保留以 `/` 开头的站内路径及其查询参数和片段；含反斜杠、控制字符、绝对 URL（包括同源绝对地址）、协议相对 URL 或无法解析的值均回到 `/`，不会导航到外部站点。

CNKI 会话按 LitRadar 用户隔离；状态接口只返回安全元数据，不返回 token 或 Cookie 值。

### 访问令牌创建规则

`POST /api/auth/tokens` 按以下固定顺序处理新令牌请求：

1. 按 OpenAPI schema 校验请求体：未裁剪 JSON `name` 最多 100 个 Unicode code points（`maxLength = 100`），`ttl` 必须处于 `3600..=31536000` 秒（省略时为 `604800`）；越界值直接拒绝。这一步先于认证。
2. 认证当前用户。
3. 裁剪首尾空白；空名称仍可创建未命名令牌，裁剪后精确等于 `login` 的名称保留给浏览器会话。
4. 在事务内检查当前用户的 active personal tokens；达到 50 个时拒绝新建。

写入事务还会复核认证时观察到的用户令牌代际，以及发起请求的确切 Bearer/Cookie token 仍属于该用户且未过期。若并发的 `logout`、`logout-all`、改密、重置或单令牌吊销先提交，新令牌不会落库，接口返回 `401 Authentication state changed; authenticate again`；客户端重新认证后可显式重试。

失败只返回当前顺序中的第一项：

- `422`：`name` 超长或 `ttl` 越界，`errors[].location` 为 `body.name` 或 `body.ttl`
- `401`：认证失败
- `400`：`Access token name "login" is reserved`
- `409`：`Active access token limit of 50 reached; revoke a token before creating another`

第一方设置界面用 `Array.from(rawName).length` 计算原始名称的 Unicode code points，并有意省略原生 HTML `maxlength`，因为后者按 UTF-16 code units 计数；服务端仍是所有客户端的权威校验方。已有超过 50 个 active personal tokens 的账号不会被迁移或删除，仍可列出、使用和撤销现有令牌，但必须降到 50 以下才能创建新令牌。

### 收藏与追踪

| 方法             | 路径                                                       | 作用                          |
| ---------------- | ---------------------------------------------------------- | ----------------------------- |
| `GET` / `POST`   | `/api/favorites/folders`                                   | 列出或创建文件夹              |
| `PUT` / `DELETE` | `/api/favorites/folders/{folder_id}`                       | 重命名或删除文件夹            |
| `GET` / `PUT`    | `/api/favorites/tracking`                                  | 查看或设置追踪文件夹          |
| `GET`            | `/api/favorites/folders/{folder_id}/articles/page`         | 收藏文章游标页                |
| `GET` / `POST`   | `/api/favorites/folders/{folder_id}/articles`              | 列出或添加收藏                |
| `DELETE`         | `/api/favorites/folders/{folder_id}/articles/{article_id}` | 删除单条收藏                  |
| `POST`           | `/api/favorites/folders/{folder_id}/articles/bulk`         | 批量添加收藏                  |
| `POST`           | `/api/favorites/folders/{folder_id}/articles/bulk-remove`  | 批量删除收藏                  |
| `POST`           | `/api/favorites/folders/{folder_id}/articles/bulk-move`    | 批量移动收藏                  |
| `GET`            | `/api/favorites/folders/{folder_id}/count`                 | 文件夹文章数                  |
| `GET`            | `/api/favorites/folders/{folder_id}/export`                | 导出引文数据                  |
| `GET`            | `/api/favorites/check`                                     | 查询一篇文章所在文件夹        |
| `POST`           | `/api/favorites/check/batch`                               | 批量查询收藏状态              |
| `GET`            | `/api/tracking/status`                                     | 当前追踪状态                  |
| `GET` / `PUT`    | `/api/tracking/notification-settings`                      | 当前用户通知设置              |
| `GET`            | `/api/tracking/ai-endpoints`                               | 管理员批准的 AI Endpoint      |
| `POST`           | `/api/tracking/push-weekly`                                | 启动当前用户的手动周报任务    |
| `GET`            | `/api/tracking/push-weekly/status`                         | 查询手动周报任务状态          |
| `GET`            | `/api/tracking/push-weekly/runs/{run_id}`                  | 按 ID 查询 owner/admin 任务   |
| `POST`           | `/api/tracking/push-weekly/runs/{run_id}/cancel`           | 请求取消 owner/admin 任务     |
| `POST`           | `/api/tracking/push-weekly/runs/{run_id}/acknowledge`      | owner 确认 Unknown 并新建任务 |

收藏 folder ID、单条删除路径的 article ID 和 tracking 的 folder ID 必须为正整数。添加及批量 item 的字符串 `article_id` 必须使用无符号、无前导零的正十进制形式。列表 `limit` 为 `1..=500`，普通列表默认 100、游标页默认 50；普通列表 `offset` 默认 0 且不得为负。导出 `format` 只接受 `bibtex`、`ris`、`endnote`，默认 `bibtex`。这些范围、格式和字符长度约束由 schema 校验，非法输入在认证和任务接纳之前返回 `422`；仅包含空白的文件夹名仍在认证后返回业务错误 `400`。

收藏文件夹名称按 Unicode scalar value 计数，最多 100 个字符；note 最多 2,000 个字符，`db_name` 最多 255 个字符。批量添加、删除、移动和检查每次最多提交 500 个 article item/ID；501 个及以上在 schema 校验阶段返回 `422`，不会构造 SQL。动态 `IN` 查询固定按 500 个 ID 分块，HTTP JSON body 超过框架的 2 MiB 上限仍返回 `413`。

收藏文章列表的每一行都包含必填的 `metadata_status`：`available` 表示索引元数据已读取，`missing` 表示来源数据库或文章已不存在，`unavailable` 表示文件系统、SQLite 或作者 JSON 等操作性读取失败。后两种状态都保留收藏行及其移动、删除能力；`missing` 条目导出为空元数据引文，`unavailable` 条目按下述原子导出规则返回明确失败。服务端只以安全数据库标识和错误类别记录 unavailable 事件，不记录 note、abstract、绝对路径或原始数据库错误。

引文导出通过 `format=bibtex|ris|endnote` 选择格式，文件扩展名和响应 Content-Type 保持为 `.bib`/`application/x-bibtex`、`.ris`/`application/x-research-info-systems` 和 `.xml`/`application/xml`。服务端使用格式专用 serializer：BibTeX 保留字符和结构性换行被编码为字段值，RIS 值被规范为单行，EndNote 只写入合法且已转义的 XML 1.0 文本；文章元数据不能注入额外字段或记录。

服务端在认证后先校验格式，再以固定的 `created_at DESC, id DESC` 顺序读取最多 10,000 条收藏，并按每批 250 条只加载标题、作者、期刊、日期和 DOI。最终 UTF-8 内容最多 8 MiB；第 10,001 条收藏或下一个会超过限制的字节都会使整个请求返回 `413`，且不会返回 attachment header 或部分文件。缺失的数据库或文章保留一条空元数据引文；文件系统、SQLite 或作者 JSON 等操作错误返回完整失败，不会伪装成成功导出。

手动周报是 SQLite 持久化异步任务。启动接口返回 `202`；没有任务时状态为 `idle`，`pending/running` 状态应继续轮询，服务重启后仍可通过 latest 或 run-id 接口恢复。公开终态为 `completed`、`failed`、`cancelled`、`timed_out` 或 `unknown`，并返回 `deadline_at`、`cancellation_requested`、`can_cancel` 和 `can_retry`。完整通知链路见[通知指南](../guides/notifications.md)。

SQLite 保证每个用户最多一个 queued/active 手动任务；同一用户重复启动返回现有 job，不同用户可以同时排队或在实例有界池中并行。普通用户只能查询和取消自己的 run；管理员可按不可猜测的 job id 管理任意用户 run。`unknown` 表示外部结果可能已发生，`can_retry=false`，客户端不得把它当普通失败自动重放。普通启动在最新任务为 Unknown 时固定返回 `409`，不创建新行。owner 检查外部投递记录后，可对该最新任务调用 `POST /api/tracking/push-weekly/runs/{run_id}/acknowledge`；服务在一个 immediate transaction 中复核 ownership/latest/Unknown，写入 `manual_push_unknown_acknowledge` 安全审计并返回一个新 queued job。畸形 `run_id`（非 32 位十六进制字符串）在认证前返回 `422`；格式正确但不存在、或不属于请求 owner 的 ID 返回 `404`。过期、重复或非 Unknown 确认返回 `409`。管理员不能代替 owner 确认。旧 run、item 和 Unknown/confirmed dedupe 保持不变，所以旧的不确定文章不会重发，后续 manifest 的新文章仍可处理。

`PUT /api/tracking/notification-settings` 的 `ai_retry_attempts` 只接受 `1..=10`。超出范围时返回 `422`，且不会替换已有设置。历史或被手工修改的数据库值在读取时会归一到该范围，但服务不会因此自动改写数据库。

`delivery_method=pushplus` 要求事务内解析出的最终 `pushplus_token` 非空；同时启用 `sync_to_tracking_folder` 还要求该用户在同一事务快照中存在追踪文件夹。保存设置与 `DELETE /api/favorites/folders/{folder_id}` 都获取认证库的 immediate 写锁：若设置先提交，删除所依赖追踪文件夹返回 `400 A tracking folder is required before enabling PushPlus sync to tracking`；若删除或 token 清除先提交，后续设置保存返回相同的依赖错误或 `400 pushplus_token is required when delivery_method is 'pushplus'`，且不会部分覆盖其他字段。

`GET /api/tracking/ai-endpoints` 需要登录，返回管理员当前批准的规范 HTTPS base URL 数组。通知设置中的非空主备 base URL 必须准确匹配该数组；不合法、未批准或已撤销的值返回固定 `400`，不会回显所提交 URL，也不会写入其他字段。

通知设置最多包含 100 个关键词、100 个研究方向和 500 个数据库；单个偏好最多 500 个字符，URL 最多 2,048，model 最多 200，system prompt 最多 10,000，PushPlus template/topic/channel 分别最多 64/200/64 个字符。公告 title/message 分别最多 200/10,000 个字符。REST 与 storage 使用同一组 Unicode 字符和 item-count 校验。

### 管理接口

| 方法             | 路径                                         | 作用                               |
| ---------------- | -------------------------------------------- | ---------------------------------- |
| `GET`            | `/api/admin/users`                           | 用户与管理面板计数                 |
| `PUT`            | `/api/admin/users/{user_id}/admin`           | 授予或撤销管理员                   |
| `POST`           | `/api/admin/users/{user_id}/reset-password`  | 重置用户密码                       |
| `DELETE`         | `/api/admin/users/{user_id}`                 | 删除用户及关联数据                 |
| `GET` / `POST`   | `/api/admin/invite-codes`                    | 列出或按策略创建邀请码             |
| `DELETE`         | `/api/admin/invite-codes/{code_id}`          | 永久撤销邀请码并保留历史           |
| `GET`            | `/api/admin/stats`                           | 管理面板统计                       |
| `GET`            | `/api/admin/telemetry`                       | 匿名按小时负载与检索形态遥测       |
| `GET` / `POST`   | `/api/admin/scheduled-tasks`                 | 列出或创建类型化计划任务           |
| `PUT` / `DELETE` | `/api/admin/scheduled-tasks/{task_id}`       | 更新或删除计划任务                 |
| `GET`            | `/api/admin/scheduler/status`                | 调度游标、内嵌调度心跳与近期运行   |
| `GET`            | `/api/admin/provider-catalog`                | Provider 聚合能力与 CSV/数据库目录 |
| `GET` / `PUT`    | `/api/admin/runtime-settings`                | 读取或更新运行时配置               |
| `GET` / `POST`   | `/api/admin/announcements`                   | 列出或创建公告                     |
| `PUT` / `DELETE` | `/api/admin/announcements/{announcement_id}` | 更新或删除公告                     |

所有管理写请求都会在目标 storage 事务取得 `BEGIN IMMEDIATE` 写锁后重新读取 actor 的当前管理员状态。若撤权先提交，即使请求已通过路由入口检查或已经完成密码 KDF/输入校验，该写入也返回固定 `403` 且不改变目标、令牌或完成审计；若写事务先提交，则撤权在其后线性化。读请求仍使用入口身份快照，不产生持久副作用。

管理员创建邀请码的 JSON body 可省略；可选字段为绝对 Unix 秒 `expires_at`（必须晚于当前时间且最多 365 天）和 `max_uses`（`1..=1000`）。省略时仍使用 7 天、1 次。删除路由保留 HTTP `DELETE` 兼容性，但只写入 `revoked_at`，不会物理删除兑换历史。

公告 `priority` 只接受精确的小写枚举 `high`、`normal`、`low`，创建时默认 `normal`；不再对优先级修剪空白或转换大小写。非法值在认证前返回 `422`。

计划任务的 `name`、`cron`、`timezone` 不接受空字符串（`422`）；仅含空白、非法 cron 语法或无效时区仍在认证后返回 `400`。创建默认 UTC、3,600 秒超时、启用和 coalesce；显式 `false` 及更新时的字段省略语义保持不变。

计划任务只接受固定的类型化 job。内嵌调度器将已验证字段转换为当前可执行文件加 `index`、`notify` 或 `push` 子命令的完整 argv，不会执行 shell 命令。应用终止时，活动子进程会被结束并等待，运行状态保存为 `cancelled`。旧 `legacy_command` 只供审阅，不能启用或执行。

`last_status` 和近期 run 的状态使用同一封闭枚举：`idle`、`pending`、`claimed`、`running`、`success`、`failed`、`timed_out`、`error`、`unknown`、`cancelled`。旧数据库中的空值映射为 `idle`，未识别旧值显式映射为 `unknown`；新的写入只能使用声明值，终结接口还会拒绝非终态。

`GET /api/admin/runtime-settings` 为每个字段返回 group、control、apply mode、allowed values、秘密状态和值来源；前端据此覆盖全部后端设置，而不是维护第二份字段清单。`GET /api/admin/provider-catalog` 按逻辑 Provider 名称聚合索引、摘要页和全文 capability，并列出从 `data/meta/*.csv` 与 `data/index/*.sqlite` 发现的安全 catalog stem/文件名。它不返回上游 URL、凭据或文件路径。

Provider 设置的内部 JSON 结构同时发布为 OpenAPI 命名 schema：`ProviderOrders` 描述两个 `article_*_provider_orders`，`IndexProviderRoutes` 描述 `index_provider_routes`，`ProviderProxyPolicy` 描述 `provider_proxy_policy`。响应 `Info` 根据 `field` 用 `if`/`then` 为 `value` 标注 `contentMediaType: application/json` 和对应的 `contentSchema` 引用；更新请求的 `values` 为这四个已知 key 提供相同标注，并保留其他设置的通用字符串值。传输仍是 JSON 字符串，更新时仍可用 `null` 清除设置；这些标注不把字段转换成对象，也不代替服务端的语义校验。前端可引用生成的 `components['schemas'][名称]` 描述解码结果，无需另写内部结构类型。

`PUT /api/admin/runtime-settings` 会在管理员认证后原子校验全部变更，包括 CORS/MCP Origin、Provider capability、Provider 顺序、日志格式和 tracing filter。无效值返回 `400`，且同一请求的任何字段都不会保存。不同字段按响应中的 apply mode 在下一请求、下一命令或进程重启后生效；完整语法及旧配置恢复边界见[配置参考](configuration.md)。

### 负载遥测

`GET /api/admin/telemetry?period=7d&kind=search` 返回管理面板「负载」分区使用的匿名负载统计（operation ID `telemetry`，响应 schema `TelemetryResponse`，生成类型见 `apps/client/frontend/lib/generated/api-schema.tsx` 中的 `TelemetryResponse`、`TelemetrySeries`、`TelemetryHour` 和 `TelemetrySearchShape`）。认证与其他管理接口相同，接受 Bearer 令牌或会话 Cookie：未认证返回 `401`，已认证的非管理员返回 `403`。

| 参数     | 取值                                  | 说明                                           |
| -------- | ------------------------------------- | ---------------------------------------------- |
| `period` | `24h`（默认）、`7d`、`30d`            | 分别对应恰好 24、168、720 个 UTC 小时桶        |
| `kind`   | 省略（全部）、`http`、`mcp`、`search` | 同时过滤 `series`、`hourly` 和 `search_shapes` |

其他取值返回 `422`。窗口按 UTC 整点对齐并包含当前未结束的小时：`from` 为闭区间起点，`to` 为下一个 UTC 整点（开区间）；不接受任意 `from`/`to` 参数。保留期短于所选时段时，更早的小时没有数据。

响应字段：

- `enabled`：采集器是否启用。`telemetry_enabled=false` 或遥测库初始化失败时为 `false`，同时返回所选窗口的 `from`/`to` 和三个空数组。
- `series`：按「类型 + 维度」合并整个窗口后的序列，按 `calls` 降序排列（相同时按维度 JSON 稳定排序）。每项展开维度字段：`kind`、`name`（HTTP 路由模板或固定未匹配标签、MCP 工具名或检索入口）、`is_error`、`uses_prefix`、`uses_boolean`，以及按类型出现的 `method`、`status_class`、`mode`、`database`、`term_count`、`hits`、`hit_count_basis`；指标为 `calls`、`errors`、`error_rate`（`0..1` 的比例）和 `p50_ms`/`p95_ms`/`p99_ms`。HTTP 的 4xx 与 5xx 都计为错误，不同状态类是不同序列，各自有独立的错误率。
- 百分位是先合并相关小时的直方图、再取最近秩所在 2 的幂微秒桶的上界，以毫秒表示，只有对数精度，不是精确观测值。
- `hourly`：稀疏的「小时 + 类型」行，按小时、再按类型升序，只包含有观测的小时；客户端应根据 `from`/`to` 补零。
- `search_shapes`：各检索特征的边际分布（不是交叉组合），每项为 `feature`、`value` 和按调用加权的 `calls`。特征为 `endpoint`、`mode`、`database`、`term_count`、`uses_prefix`、`uses_boolean`、`hits` 和 `hit_count_basis`；只请求 `http` 或 `mcp` 时为空数组。`hits` 桶的计数口径由 `hit_count_basis` 区分：`total` 为总命中数，`returned` 为无总数时的返回条数，`candidates` 为相关推荐去重前的候选数。不同口径代表不同总体，不能合并解读；需要按口径拆分时使用 `series` 中的 `hits` 与 `hit_count_basis` 组合。

所有数组在为空时都以 `[]` 出现。读取遥测存储失败时返回 `503`，body 为 `{"title":"Service Unavailable","status":503,"detail":"Load telemetry temporarily unavailable"}`（`application/problem+json`），并带 `Retry-After: 5`。当前这次 `/api/admin/telemetry` 请求在自身响应快照之后才被记录，因此出现在下一次读取中；成功的健康检查同样会被计量，尽管请求日志不记录它们。数值为尽力而为的估计：锁竞争、序列上限或存储失败都可能导致少计，写入与保留语义见[数据库参考](database.md#负载遥测库)。

隐私边界：采集器不接收、也不存储用户 ID、用户名、IP、授权令牌、request ID、原始路径、原始查询字符串或检索/标题文本；不提供按用户聚合或原始检索记录。项目也不提供 Prometheus `/metrics` 端点或 CPU profile 采集端点。开关和保留期见[配置参考](configuration.md#负载遥测设置)。

## 文章访问边界

`GET /api/articles/{article_id}/access` 是前端决定按钮文案与本地可用性的权威接口。它只读取：

- 文章是否存在；
- 当前数据库 stem 对应的摘要页和全文 Provider 顺序；
- 已注册 capability；
- 当前用户是否已有 active ZJLib CNKI 会话。

它不请求上游，也不返回 Provider 名称或目的 URL。响应始终只包含 `abstract_page` 和 `fulltext` 两个动作，每个动作只有 `available`、`label`、`requires_login` 和可选 `message`。

动作路由在用户调用时才加载 Provider-neutral `ArticleLocator` 并在线解析。每个动作先查当前 catalog override；没有 override 时继承 default，显式空数组则禁用该动作。列表是有序 fallback，不是索引来源：例如默认摘要顺序 `scholarly → cnki` 会先尝试 scholarly，遇到超时、未找到、临时失败或无效结果才继续 CNKI。未注册或未声明能力的名称被忽略。全部失败后才返回 `404`，或在只剩认证阻断时返回结构化 `428 article_access_authentication_required`。

前端“文章详情”弹窗只显示 LitRadar 已存的规范文章字段和摘要文本；它不是在线 Provider capability，也不会发起第三种文章动作。在线“查看摘要页”才调用 `/abstract` 并可能跳转到外部站点。

安全边界：

- redirect 只允许 HTTPS、无 user-info/控制字符且精确匹配 Provider 注册的运行时 host allowlist；
- 全文文档只接受非空 `application/pdf`，最大 32 MiB，文件名必须是安全 basename；
- 成功 redirect 使用 `307 Temporary Redirect`；PDF 使用 attachment 响应；两者都设置 `Cache-Control: private, no-store`；
- 上游 URL、文档、访问结果和时间不写入内容库、控制库、认证库或文件缓存；
- ZJLib 全文读取当前用户会话，但动作不会更新或 touch 会话；候选题名、作者和期刊必须精确规范匹配。

前端只生成 LitRadar 自己的 `/abstract` 和 `/fulltext` URL，不直接使用索引字段或上游 permalink。DOI 仍是规范文章标识，可用于复制和生成引文。Provider 接入规范见[索引与 Provider 契约](index-provider-contract.md)。

## 缓存与 CORS

所有 REST、健康检查、OpenAPI 和 MCP 响应都带有同一基线安全 Header：严格 CSP、`nosniff`、`Referrer-Policy: same-origin`、禁用敏感浏览器能力以及双重 frame denial。`/docs` 文档页例外地保留 huma 为它设置的 CSP，只额外允许从 unpkg.com 加载固定版本的 Stoplight Elements 脚本与样式，其余 Header 相同。CSP 的 `script-src` 为同源外部脚本；Vite UI 由独立 Wails Web host 提供，不依赖 HTML 内联脚本哈希。hardened HTTPS 服务端发送一年 HSTS。

`/api/articles*`、`/api/meta*` 及其他受保护路由需要普通用户或管理员身份，不能作为匿名共享缓存内容。请求带有 `Authorization` 或 `litradar_session` 时，以及任何返回 `401 Unauthorized` 的响应，后端都会设置：

```http
Cache-Control: private, no-store
```

认证命名空间采用独立且更严格的路径规则。`/api/auth` 与 `/api/auth/*` 的所有响应（包括 `200`、`400`、`401`、`429` 和 `500`）都会覆盖为：

```http
Cache-Control: no-store
Pragma: no-cache
```

前文列出的免认证端点在成功响应时保持现有缓存头行为；本策略不会为它们新增共享缓存 TTL。

Web 浏览器经 Caddy 同源访问独立 Web host 和 `/api/*`；开发时 Vite 8000 代理到内部 Go 8001。原生 Windows/Android 客户端使用用户配置的服务端 URL 和 Bearer token。其他跨源浏览器客户端必须在 `cors_allowed_origins` 显式列出 Origin；不要使用通配 Origin 搭配 Cookie。

Web host 对 `/assets/*` 哈希资源使用长期 immutable 缓存，SPA HTML 重新验证；服务端不提供静态 UI。Caddy 保留后端命名空间的路由优先级。

## Streamable HTTP MCP

`GET`、`POST` 和 `DELETE /mcp` 提供 Streamable HTTP MCP 传输。它复用 REST 的 Bearer 或会话 Cookie 认证，但不属于 `/api`，也不进入 OpenAPI。

当前工具：

| 领域   | 工具                                                                                         |
| ------ | -------------------------------------------------------------------------------------------- |
| 元数据 | `list_databases`、`list_areas`、`list_years`、`list_journal_options`、`list_journal_ratings` |
| 期刊   | `list_journals`、`get_journal`                                                               |
| 文章   | `search_articles`、`get_article`                                                             |
| 更新   | `get_weekly_updates`                                                                         |
| 收藏   | `list_folders`、`add_favorite`、`remove_favorite`                                            |

工具结果的 text content 是 JSON 字符串。收藏工具始终使用当前认证用户 ID，不能访问其他用户的数据。所有 MCP 字符串参数最多 2,048 个 Unicode 字符，数组参数最多 500 项；超限作为 tool-level error 返回，不进入 SQLite 查询。

`mcp_allowed_hosts` 默认只允许本机 host；经公网域名、局域网地址或反向代理访问时必须显式配置。浏览器跨源调用 MCP 时再设置 `mcp_allowed_origins`。详见[配置参考](configuration.md)。

## 原生客户端登录

`POST /api/auth/native/login` 接受 username、password、device_name，返回 token、expires_at 与 user，不写浏览器 Cookie。服务端签发名称以 `device:` 开头、90 天有效的独立个人令牌；使用 Authorization Bearer 认证。访问令牌列表可撤销，原生 logout 撤销当前令牌。原生客户端只支持账号密码和 OIDC 登录；原生 OIDC 使用 `/api/auth/oidc/native/login` 与 `/api/auth/oidc/native/exchange`，见[安全说明](../operations/security.md#原生客户端-oidc)。见[客户端指南](../guides/clients.md)。
