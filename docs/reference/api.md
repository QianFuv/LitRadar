# API 参考

本文档补充 OpenAPI 不便表达的认证方式、跨接口约定与业务边界。字段、请求体和响应 schema 以运行中的 OpenAPI 为准：

- Swagger UI：`/docs/`
- OpenAPI JSON：`/openapi.json`
- 前端生成基线：`app/lib/generated/openapi.json`

Rust handler 上的 OpenAPI 注解是 REST 契约的实现来源。修改 REST 接口后，应重新生成前端基线；不要在本文重复维护完整 schema。

## 地址与认证

本地默认地址为 `http://localhost:8000`。同一 Rust 监听器还提供 Web 根路径、`/docs/`、`/openapi.json` 和 `/mcp`，REST 路径统一以 `/api` 开头。支持两种认证方式：

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

全文查询 `q` 默认使用 `search_mode=simple`，把完整输入转义为一个 FTS5 字面短语，引号和 `OR` 等符号不会被解释为运算符。只有显式设置 `search_mode=advanced` 才启用 FTS5 查询语法；非法高级表达式返回 `400 Invalid search expression`。REST、MCP 与 storage 均限制搜索文本最多 2048 个 Unicode 字符、重复 `journal_id`/`area` 过滤值合计最多 500 项。

### 错误

普通错误响应采用统一形状：

```json
{
  "detail": "Readable error message"
}
```

常见状态码：

| 状态码 | 含义                                           |
| ------ | ---------------------------------------------- |
| `400`  | 参数、数据库选择或业务输入无效                 |
| `401`  | 缺少凭据、会话失效或 Bearer 格式错误           |
| `403`  | 当前用户没有管理员权限                         |
| `404`  | 数据库或记录不存在                             |
| `409`  | 用户名、文件夹名等唯一约束冲突                 |
| `428`  | 所有可用在线 Provider 都要求先完成认证         |
| `429`  | 认证请求触发进程内限流；响应包含 `Retry-After` |
| `503`  | 内嵌调度尚未 ready 或服务暂时不可用            |

服务端不会在通用 `500` / `503` 响应中暴露内部错误细节。

## REST 端点目录

本节只提供导航和职责边界。参数与响应字段请直接查看 Swagger UI。

### 健康、公告与索引读取

| 方法  | 路径                                  | 作用                             |
| ----- | ------------------------------------- | -------------------------------- |
| `GET` | `/health/live`                        | 应用事件循环存活状态             |
| `GET` | `/health/ready`                       | 最近 90 秒内是否存在内嵌调度心跳 |
| `GET` | `/api/announcements`                  | 当前启用的公告                   |
| `GET` | `/api/meta/databases`                 | 可用索引库                       |
| `GET` | `/api/meta/areas`                     | 领域与数量                       |
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

`weekly-updates` 读取 `data/push_state/*.changes.json` 中的可通知文章，不会临时重新抓取数据。

期次、文章和 weekly article 的 `date` 只使用经真实日历校验的 `YYYY`、`YYYY-MM` 或 `YYYY-MM-DD`；配套 `date_precision` 为 `year`、`month`、`day` 或 `null`。只有年份的上游元数据保持如 `2026`，不会补成 `2026-01-01`。历史库中无法通过日历校验的旧日期仍原样可读，但其 precision 为 `null`。

健康响应的 `status` 只可能是 `ok` 或 `unhealthy`。应用自有 manual、scheduler 和 push-stat 状态同样由 OpenAPI enum 约束；服务不会返回任意拼写。CNKI 上游状态不属于该封闭集合，未知字符串会原样返回，客户端必须保留兼容分支。

### 认证与 CNKI 会话

| 方法             | 路径                          | 作用                             |
| ---------------- | ----------------------------- | -------------------------------- |
| `POST`           | `/api/auth/register`          | 使用邀请码注册普通用户           |
| `POST`           | `/api/auth/login`             | 登录并设置会话 Cookie            |
| `GET`            | `/api/auth/invite-required`   | 注册与首管理员初始化状态         |
| `GET`            | `/api/auth/me`                | 当前用户                         |
| `POST`           | `/api/auth/change-password`   | 修改当前用户密码                 |
| `POST`           | `/api/auth/logout`            | 注销当前会话                     |
| `POST`           | `/api/auth/logout-all`        | 撤销当前用户的全部会话与访问令牌 |
| `GET` / `POST`   | `/api/auth/tokens`            | 列出或创建访问令牌               |
| `DELETE`         | `/api/auth/tokens/{token_id}` | 吊销访问令牌                     |
| `GET` / `POST` / `DELETE` | `/api/auth/invite-code`       | 查看、生成或永久撤销当前用户的邀请码 |
| `POST`           | `/api/auth/invite-code/rotate` | 撤销旧邀请码并原子签发替代码     |
| `GET` / `DELETE` | `/api/cnki/session`           | 查看或清除当前用户的 CNKI 会话   |
| `POST`           | `/api/cnki/login/start`       | 启动浙江图书馆扫码登录           |
| `POST`           | `/api/cnki/login/poll`        | 轮询扫码登录状态                 |

公开注册始终要求有效邀请码，且只能创建非管理员。首个管理员必须在本机通过 `litradar admin bootstrap` 创建，API 不提供远程引导端点。新密码至少为 12 个 Unicode 字符。

普通用户邀请码默认有效 7 天、最多注册 1 次。`GET` 返回 `status=active|expired|revoked|exhausted`、`expires_at`、`revoked_at`、`max_uses` 与 `use_count`；`rotate` 在一个事务中永久撤销当前未撤销码并创建替代码。过期、撤销或用尽的邀请码均不能注册，重复撤销返回 `404`。

浏览器登录 Cookie 使用固定 7 天有效期，每次登录轮换，不因普通 API 访问而滚动延长。登录写入会原子复核密码验证时观察到的令牌代际；若改密、管理员重置或 `logout-all` 已先提交，旧验证结果只会得到认证失败，不能创建撤销后的新 Cookie。`POST /api/auth/logout` 只撤销当前凭据；`POST /api/auth/logout-all` 在一个事务中递增令牌代际并撤销该用户的浏览器登录令牌和全部 Personal Access Token。两个端点对携带 `litradar_session` 的请求无论成功或失败都返回清除 Cookie 的 `Set-Cookie`；`logout` 返回 `401` 表示请求到达前令牌已失效，第一方浏览器将其视为幂等注销完成。SQLite busy/locked 只执行一次短时重试；若持久删除仍未确认，返回 `503`：

登录页的 `next` 查询参数会按 URL 规则规范化，只有同源站内路径会保留 pathname、query 和 fragment。含反斜杠或控制字符的值、绝对 URL、协议相对 URL 以及无法解析的值均视为 `/`，不会在登录前后导航到外部站点。

```json
{
  "detail": {
    "code": "session_revocation_unconfirmed",
    "message": "Session revocation could not be confirmed",
    "request_id": "server-generated-request-id"
  }
}
```

浏览器此时必须清除非秘密本地用户快照，但不能声称服务端令牌已经撤销；第一方界面会保留跨刷新的警告和 request ID，并要求重新输入账号密码。重新认证取得一个新 Cookie 后，界面立即调用 `/api/auth/logout-all`，而不是尝试重放已清除的旧 Cookie。只有该请求成功后才清除警告。

CNKI 会话按 LitRadar 用户隔离；状态接口只返回安全元数据，不返回 token 或 Cookie 值。

#### 访问令牌创建规则

`POST /api/auth/tokens` 先认证当前用户，再按以下固定顺序处理新令牌请求：

1. 检查未裁剪 JSON `name` 的 Unicode code points 数，最多 100；OpenAPI `maxLength = 100` 约束同一个原始字符串。
2. 裁剪首尾空白；空名称仍可创建未命名令牌，裁剪后精确等于 `login` 的名称保留给浏览器会话。
3. 检查 `ttl` 是否处于 `3600..=31536000` 秒；越界值直接拒绝，不会再静默 clamp。
4. 在事务内检查当前用户的 active personal tokens；达到 50 个时拒绝新建。

写入事务还会复核认证时观察到的用户令牌代际，以及发起请求的确切 Bearer/Cookie token 仍属于该用户且未过期。若并发的 `logout`、`logout-all`、改密、重置或单令牌吊销先提交，新令牌不会落库，接口返回 `401 Authentication state changed; authenticate again`；客户端重新认证后可显式重试。

认证失败仍优先返回 `401`。其余失败只返回当前顺序中的第一项：

- `400`：`Access token name must be at most 100 Unicode code points`
- `400`：`Access token name "login" is reserved`
- `400`：`Access token TTL must be between 3600 and 31536000 seconds`
- `409`：`Active access token limit of 50 reached; revoke a token before creating another`

第一方设置界面用 `Array.from(rawName).length` 计算原始名称的 Unicode code points，并有意省略原生 HTML `maxlength`，因为后者按 UTF-16 code units 计数；服务端仍是所有客户端的权威校验方。已有超过 50 个 active personal tokens 的账号不会被迁移或删除，仍可列出、使用和撤销现有令牌，但必须降到 50 以下才能创建新令牌。

### 收藏与追踪

| 方法             | 路径                                                       | 作用                       |
| ---------------- | ---------------------------------------------------------- | -------------------------- |
| `GET` / `POST`   | `/api/favorites/folders`                                   | 列出或创建文件夹           |
| `PUT` / `DELETE` | `/api/favorites/folders/{folder_id}`                       | 重命名或删除文件夹         |
| `GET` / `PUT`    | `/api/favorites/tracking`                                  | 查看或设置追踪文件夹       |
| `GET` / `POST`   | `/api/favorites/folders/{folder_id}/articles`              | 列出或添加收藏             |
| `DELETE`         | `/api/favorites/folders/{folder_id}/articles/{article_id}` | 删除单条收藏               |
| `POST`           | `/api/favorites/folders/{folder_id}/articles/bulk`         | 批量添加收藏               |
| `POST`           | `/api/favorites/folders/{folder_id}/articles/bulk-remove`  | 批量删除收藏               |
| `POST`           | `/api/favorites/folders/{folder_id}/articles/bulk-move`    | 批量移动收藏               |
| `GET`            | `/api/favorites/folders/{folder_id}/count`                 | 文件夹文章数               |
| `GET`            | `/api/favorites/folders/{folder_id}/export`                | 导出引文数据               |
| `GET`            | `/api/favorites/check`                                     | 查询一篇文章所在文件夹     |
| `POST`           | `/api/favorites/check/batch`                               | 批量查询收藏状态           |
| `GET`            | `/api/tracking/status`                                     | 当前追踪状态               |
| `GET` / `PUT`    | `/api/tracking/notification-settings`                      | 当前用户通知设置           |
| `GET`            | `/api/tracking/ai-endpoints`                               | 管理员批准的 AI Endpoint   |
| `POST`           | `/api/tracking/push-weekly`                                | 启动当前用户的手动周报任务 |
| `GET`            | `/api/tracking/push-weekly/status`                         | 查询手动周报任务状态       |
| `GET`            | `/api/tracking/push-weekly/runs/{run_id}`                  | 按 ID 查询 owner/admin 任务 |
| `POST`           | `/api/tracking/push-weekly/runs/{run_id}/cancel`           | 请求取消 owner/admin 任务   |
| `POST`           | `/api/tracking/push-weekly/runs/{run_id}/acknowledge`      | owner 确认 Unknown 并新建任务 |

收藏文件夹名称按 Unicode scalar value 计数，最多 100 个字符；note 最多 2,000 个字符，`db_name` 最多 255 个字符。批量添加、删除、移动和检查每次最多提交 500 个 article item/ID；501 个及以上在构造 SQL 前返回 `400`。动态 `IN` 查询固定按 500 个 ID 分块，HTTP JSON body 超过框架的 2 MiB 上限仍返回 `413`。

收藏文章列表的每一行都包含必填的 `metadata_status`：`available` 表示索引元数据已读取，`missing` 表示来源数据库或文章已不存在，`unavailable` 表示文件系统、SQLite 或作者 JSON 等操作性读取失败。后两种状态都保留收藏行及其移动、删除能力；`missing` 条目导出为空元数据引文，`unavailable` 条目按下述原子导出规则返回明确失败。服务端只以安全数据库标识和错误类别记录 unavailable 事件，不记录 note、abstract、绝对路径或原始数据库错误。

引文导出通过 `format=bibtex|ris|endnote` 选择格式，文件扩展名和响应 Content-Type 保持为 `.bib`/`application/x-bibtex`、`.ris`/`application/x-research-info-systems` 和 `.xml`/`application/xml`。服务端使用格式专用 serializer：BibTeX 保留字符和结构性换行被编码为字段值，RIS 值被规范为单行，EndNote 只写入合法且已转义的 XML 1.0 文本；文章元数据不能注入额外字段或记录。

服务端在认证后先校验格式，再以固定的 `created_at DESC, id DESC` 顺序读取最多 10,000 条收藏，并按每批 250 条只加载标题、作者、期刊、日期和 DOI。最终 UTF-8 内容最多 8 MiB；第 10,001 条收藏或下一个会超过限制的字节都会使整个请求返回 `413`，且不会返回 attachment header 或部分文件。缺失的数据库或文章保留一条空元数据引文；文件系统、SQLite 或作者 JSON 等操作错误返回完整失败，不会伪装成成功导出。

手动周报是 SQLite 持久化异步任务。启动接口返回 `202`；`pending/running` 状态应继续轮询，服务重启后仍可通过 latest 或 run-id 接口恢复。公开终态为 `completed`、`failed`、`cancelled`、`timed_out` 或 `unknown`，并返回 `deadline_at`、`cancellation_requested`、`can_cancel` 和 `can_retry`。完整通知链路见[通知指南](../guides/notifications.md)。

SQLite 保证每个用户最多一个 queued/active 手动任务；同一用户重复启动返回现有 job，不同用户可以同时排队或在实例有界池中并行。普通用户只能查询和取消自己的 run；管理员可按不可猜测的 job id 管理任意用户 run。`unknown` 表示外部结果可能已发生，`can_retry=false`，客户端不得把它当普通失败自动重放。普通启动在最新任务为 Unknown 时固定返回 `409`，不创建新行。owner 检查外部投递记录后，可对该最新任务调用 `POST /api/tracking/push-weekly/runs/{run_id}/acknowledge`；服务在一个 immediate transaction 中复核 ownership/latest/Unknown，写入 `manual_push_unknown_acknowledge` 安全审计并返回一个新 queued job。非 owner 或畸形 ID 返回 `404`，过期、重复或非 Unknown 确认返回 `409`。管理员不能代替 owner 确认。旧 run、item 和 Unknown/confirmed dedupe 保持不变，所以旧的不确定文章不会重发，后续 manifest 的新文章仍可处理。

`PUT /api/tracking/notification-settings` 的 `ai_retry_attempts` 只接受 `1..=10`。超出范围时返回 `400`，且不会替换已有设置。历史或被手工修改的数据库值在读取时会归一到该范围，但服务不会因此自动改写数据库。

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
| `GET` / `POST`   | `/api/admin/scheduled-tasks`                 | 列出或创建类型化计划任务           |
| `PUT` / `DELETE` | `/api/admin/scheduled-tasks/{task_id}`       | 更新或删除计划任务                 |
| `GET`            | `/api/admin/scheduler/status`                | 调度游标、内嵌调度心跳与近期运行   |
| `GET`            | `/api/admin/provider-catalog`                | Provider 聚合能力与 CSV/数据库目录 |
| `GET` / `PUT`    | `/api/admin/runtime-settings`                | 读取或更新运行时配置               |
| `GET` / `POST`   | `/api/admin/announcements`                   | 列出或创建公告                     |
| `PUT` / `DELETE` | `/api/admin/announcements/{announcement_id}` | 更新或删除公告                     |

所有管理写请求都会在目标 storage 事务取得 `BEGIN IMMEDIATE` 写锁后重新读取 actor 的当前管理员状态。若撤权先提交，即使请求已通过路由入口检查或已经完成密码 KDF/输入校验，该写入也返回固定 `403` 且不改变目标、令牌或完成审计；若写事务先提交，则撤权在其后线性化。读请求仍使用入口身份快照，不产生持久副作用。

管理员创建邀请码的 JSON body 可省略；可选字段为绝对 Unix 秒 `expires_at`（必须晚于当前时间且最多 365 天）和 `max_uses`（`1..=1000`）。省略时仍使用 7 天、1 次。删除路由保留 HTTP `DELETE` 兼容性，但只写入 `revoked_at`，不会物理删除兑换历史。

计划任务只接受固定的类型化 job。内嵌调度器将已验证字段转换为当前可执行文件加 `index`、`notify` 或 `push` 子命令的完整 argv，不会执行 shell 命令。应用终止时，活动子进程会被结束并等待，运行状态保存为 `cancelled`。旧 `legacy_command` 只供审阅，不能启用或执行。

`last_status` 和近期 run 的状态使用同一封闭枚举：`idle`、`pending`、`claimed`、`running`、`success`、`failed`、`timed_out`、`error`、`unknown`、`cancelled`。旧数据库中的空值映射为 `idle`，未识别旧值显式映射为 `unknown`；新的写入只能使用声明值，终结接口还会拒绝非终态。

`GET /api/admin/runtime-settings` 为每个字段返回 group、control、apply mode、allowed values、秘密状态和值来源；前端据此覆盖全部后端设置，而不是维护第二份字段清单。`GET /api/admin/provider-catalog` 按逻辑 Provider 名称聚合索引、摘要页和全文 capability，并列出从 `data/meta/*.csv` 与 `data/index/*.sqlite` 发现的安全 catalog stem/文件名。它不返回上游 URL、凭据或文件路径。

`PUT /api/admin/runtime-settings` 会在管理员认证后原子校验全部变更，包括 CORS/MCP Origin、Provider capability、Provider 顺序、日志格式和 tracing filter。无效值返回 `400`，且同一请求的任何字段都不会保存。不同字段按响应中的 apply mode 在下一请求、下一命令或进程重启后生效；完整语法及旧配置恢复边界见[配置参考](configuration.md)。

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

所有 HTML、REST、健康检查、Swagger/OpenAPI 和 MCP 响应都带有同一基线安全 Header：严格 CSP、`nosniff`、`Referrer-Policy: same-origin`、禁用敏感浏览器能力以及双重 frame denial。CSP 的 `script-src` 只包含 `'self'` 与启动时从部署 HTML 复算并核对构建清单的 SHA-256，不允许任意内联脚本；当前静态样式保留最小的 `style-src 'self' 'unsafe-inline'`。hardened HTTPS 启动模式额外返回一年 HSTS，普通 loopback HTTP 不返回 HSTS。

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

生产 Web 由 Rust 从 `/app/web` 直接提供，浏览器同源访问 `/api/*`，不依赖 Next.js 运行时或 rewrite。只有本地开发的 Next.js 8000 入口会把后端命名空间代理到内部 Rust 8001。浏览器跨源直连时，管理员必须在 `cors_allowed_origins` 中显式列出 Origin；不要使用通配 Origin 搭配 Cookie credentials。

成功的 `/_next/static/*` 哈希文件使用 `public, max-age=31536000, immutable`；页面、导航 payload 和导出的 404 使用 `no-cache`。客户端声明支持 gzip 时，Rust 优先返回镜像内预压缩文件并保留正确 MIME；原文件仍供不支持 gzip 的客户端和 Range 请求使用。后端保留 `/api`、`/mcp`、`/docs` 和 `/openapi.json` 的路由优先级。

## Streamable HTTP MCP

`GET`、`POST` 和 `DELETE /mcp` 提供 Streamable HTTP MCP 传输。它复用 REST 的 Bearer 或会话 Cookie 认证，但不属于 `/api`，也不进入 OpenAPI。

当前工具：

| 领域   | 工具                                                                 |
| ------ | -------------------------------------------------------------------- |
| 元数据 | `list_databases`、`list_areas`、`list_years`、`list_journal_options` |
| 期刊   | `list_journals`、`get_journal`                                       |
| 文章   | `search_articles`、`get_article`                                     |
| 更新   | `get_weekly_updates`                                                 |
| 收藏   | `list_folders`、`add_favorite`、`remove_favorite`                    |

工具结果的 text content 是 JSON 字符串。收藏工具始终使用当前认证用户 ID，不能访问其他用户的数据。所有 MCP 字符串参数最多 2,048 个 Unicode 字符，数组参数最多 500 项；超限作为 tool-level error 返回，不进入 SQLite 查询。

`mcp_allowed_hosts` 默认只允许本机 host；经公网域名、局域网地址或反向代理访问时必须显式配置。浏览器跨源调用 MCP 时再设置 `mcp_allowed_origins`。详见[配置参考](configuration.md)。
