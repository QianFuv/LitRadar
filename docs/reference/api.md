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

- `cursor` 格式为 `{date}|{article_id}`。
- `include_total=false` 会跳过总数查询，此时 `page.total` 可以为 `null`。
- 精确的默认值、上限和过滤字段以 OpenAPI schema 为准。

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
| `GET` | `/api/meta/sources`                   | 元数据来源与数量                 |
| `GET` | `/api/years`                          | 出版年份汇总                     |
| `GET` | `/api/journals`                       | 期刊列表                         |
| `GET` | `/api/journals/{journal_id}`          | 单个期刊                         |
| `GET` | `/api/issues`                         | 期次列表                         |
| `GET` | `/api/issues/{issue_id}`              | 单个期次                         |
| `GET` | `/api/articles`                       | 文章过滤、全文检索与分页         |
| `GET` | `/api/articles/{article_id}`          | 单篇文章                         |
| `GET` | `/api/articles/{article_id}/access`   | 当前用户可用的详情与全文动作     |
| `GET` | `/api/articles/{article_id}/fulltext` | 执行全文动作或安全重定向         |
| `GET` | `/api/weekly-updates`                 | 按数据库和期刊聚合变更清单       |

`weekly-updates` 读取 `data/push_state/*.changes.json` 中的可通知文章，不会临时重新抓取数据。

### 认证与 CNKI 会话

| 方法             | 路径                          | 作用                             |
| ---------------- | ----------------------------- | -------------------------------- |
| `POST`           | `/api/auth/register`          | 使用邀请码注册普通用户           |
| `POST`           | `/api/auth/login`             | 登录并设置会话 Cookie            |
| `GET`            | `/api/auth/invite-required`   | 注册与首管理员初始化状态         |
| `GET`            | `/api/auth/me`                | 当前用户                         |
| `POST`           | `/api/auth/change-password`   | 修改当前用户密码                 |
| `POST`           | `/api/auth/logout`            | 注销当前会话                     |
| `GET` / `POST`   | `/api/auth/tokens`            | 列出或创建访问令牌               |
| `DELETE`         | `/api/auth/tokens/{token_id}` | 吊销访问令牌                     |
| `GET` / `POST`   | `/api/auth/invite-code`       | 查看或生成当前用户的一次性邀请码 |
| `GET` / `DELETE` | `/api/cnki/session`           | 查看或清除当前用户的 CNKI 会话   |
| `POST`           | `/api/cnki/login/start`       | 启动浙江图书馆扫码登录           |
| `POST`           | `/api/cnki/login/poll`        | 轮询扫码登录状态                 |

公开注册始终要求有效邀请码，且只能创建非管理员。首个管理员必须在本机通过 `litradar admin bootstrap` 创建，API 不提供远程引导端点。新密码至少为 12 个 Unicode 字符。

CNKI 会话按 LitRadar 用户隔离；状态接口只返回安全元数据，不返回 token 或 Cookie 值。

#### 访问令牌创建规则

`POST /api/auth/tokens` 先认证当前用户，再按以下固定顺序处理新令牌请求：

1. 检查未裁剪 JSON `name` 的 Unicode code points 数，最多 100；OpenAPI `maxLength = 100` 约束同一个原始字符串。
2. 裁剪首尾空白；空名称仍可创建未命名令牌，裁剪后精确等于 `login` 的名称保留给浏览器会话。
3. 检查 `ttl` 是否处于 `3600..=31536000` 秒；越界值直接拒绝，不会再静默 clamp。
4. 在事务内检查当前用户的 active personal tokens；达到 50 个时拒绝新建。

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
| `POST`           | `/api/tracking/push-weekly`                                | 启动当前用户的手动周报任务 |
| `GET`            | `/api/tracking/push-weekly/status`                         | 查询手动周报任务状态       |

手动周报是异步任务；启动接口返回后，应通过状态接口查询进展。完整通知链路见[通知指南](../guides/notifications.md)。

同一 `litradar serve` 进程对每个 storage instance 同时只接纳一个 running manual `push-weekly` job。同一用户重复启动返回现有状态；另一用户竞争该 slot 时，`POST /api/tracking/push-weekly` 返回通用 `503` ErrorEnvelope，不创建该用户的 job。当前 job 完成或失败后可以重试；该限制不是队列或 `cross-process` 锁。

`PUT /api/tracking/notification-settings` 的 `ai_retry_attempts` 只接受 `1..=10`。超出范围时返回 `400`，且不会替换已有设置。历史或被手工修改的数据库值在读取时会归一到该范围，但服务不会因此自动改写数据库。

### 管理接口

| 方法             | 路径                                         | 作用                             |
| ---------------- | -------------------------------------------- | -------------------------------- |
| `GET`            | `/api/admin/users`                           | 用户与管理面板计数               |
| `PUT`            | `/api/admin/users/{user_id}/admin`           | 授予或撤销管理员                 |
| `POST`           | `/api/admin/users/{user_id}/reset-password`  | 重置用户密码                     |
| `DELETE`         | `/api/admin/users/{user_id}`                 | 删除用户及关联数据               |
| `GET` / `POST`   | `/api/admin/invite-codes`                    | 列出或创建邀请码                 |
| `DELETE`         | `/api/admin/invite-codes/{code_id}`          | 删除未使用的邀请码               |
| `GET`            | `/api/admin/stats`                           | 管理面板统计                     |
| `GET` / `POST`   | `/api/admin/scheduled-tasks`                 | 列出或创建类型化计划任务         |
| `PUT` / `DELETE` | `/api/admin/scheduled-tasks/{task_id}`       | 更新或删除计划任务               |
| `GET`            | `/api/admin/scheduler/status`                | 调度游标、内嵌调度心跳与近期运行 |
| `GET` / `PUT`    | `/api/admin/runtime-settings`                | 读取或更新运行时配置             |
| `GET` / `POST`   | `/api/admin/announcements`                   | 列出或创建公告                   |
| `PUT` / `DELETE` | `/api/admin/announcements/{announcement_id}` | 更新或删除公告                   |

计划任务只接受固定的类型化 job。内嵌调度器将已验证字段转换为当前可执行文件加 `index`、`notify` 或 `push` 子命令的完整 argv，不会执行 shell 命令。应用终止时，活动子进程会被结束并等待，运行状态保存为 `cancelled`。旧 `legacy_command` 只供审阅，不能启用或执行。

`PUT /api/admin/runtime-settings` 会在管理员认证后校验发生变化的 CORS/MCP Origin。无效 Origin 返回 `400`，且同一请求的任何字段都不会保存；有效设置仍在下次 `litradar serve` 启动时生效，不会热加载。完整语法及旧配置恢复边界见[配置参考](configuration.md#origin-语法)。

## 文章访问边界

`GET /api/articles/{article_id}/access` 是前端决定按钮文案与可用性的权威接口：

- 非 CNKI 文章可依据安全的上游 OA 链接提供全文动作。
- CNKI 详情链接与机构授权全文是两种不同能力。
- 只有当前用户的浙江图书馆 CNKI 会话处于 active 状态时，CNKI 全文动作才可用。
- 全文处理会校验候选文章的题名、作者和期刊，避免返回相似题名的错误 PDF。

前端应先读取 `/access`，不要自行根据数据库字段猜测全文权限。数据源侧的具体落库语义见[数据源参考](sources/)。

## 缓存与 CORS

`/api/articles*`、`/api/meta*` 及其他受保护路由需要普通用户或管理员身份，不能作为匿名共享缓存内容。请求带有 `Authorization` 或 `litradar_session` 时，以及任何返回 `401 Unauthorized` 的响应，后端都会设置：

```http
Cache-Control: private, no-store
```

前文列出的免认证端点在成功响应时保持现有缓存头行为；本策略不会为它们新增共享缓存 TTL。

生产 Web 由 Rust 从 `/app/web` 直接提供，浏览器同源访问 `/api/*`，不依赖 Next.js 运行时或 rewrite。只有本地开发的 Next.js 8000 入口会把后端命名空间代理到内部 Rust 8001。浏览器跨源直连时，管理员必须在 `cors_allowed_origins` 中显式列出 Origin；不要使用通配 Origin 搭配 Cookie credentials。

成功的 `/_next/static/*` 哈希文件使用 `public, max-age=31536000, immutable`；页面、导航 payload 和导出的 404 使用 `no-cache`。客户端声明支持 gzip 时，Rust 优先返回镜像内预压缩文件并保留正确 MIME；原文件仍供不支持 gzip 的客户端和 Range 请求使用。后端保留 `/api`、`/mcp`、`/docs` 和 `/openapi.json` 的路由优先级。

## Streamable HTTP MCP

`GET`、`POST` 和 `DELETE /mcp` 提供 Streamable HTTP MCP 传输。它复用 REST 的 Bearer 或会话 Cookie 认证，但不属于 `/api`，也不进入 OpenAPI。

当前工具：

| 领域   | 工具                                                                                 |
| ------ | ------------------------------------------------------------------------------------ |
| 元数据 | `list_databases`、`list_areas`、`list_years`、`list_journal_options`、`list_sources` |
| 期刊   | `list_journals`、`get_journal`                                                       |
| 文章   | `search_articles`、`get_article`                                                     |
| 更新   | `get_weekly_updates`                                                                 |
| 收藏   | `list_folders`、`add_favorite`、`remove_favorite`                                    |

工具结果的 text content 是 JSON 字符串。收藏工具始终使用当前认证用户 ID，不能访问其他用户的数据。

`mcp_allowed_hosts` 默认只允许本机 host；经公网域名、局域网地址或反向代理访问时必须显式配置。浏览器跨源调用 MCP 时再设置 `mcp_allowed_origins`。详见[配置参考](configuration.md)。
