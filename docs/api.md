# API 参考

本文档以当前 Rust API 实现为准，覆盖检索接口、认证接口、收藏/追踪接口以及管理员接口。后端部署与正常运行入口已经切换到 Rust；认证初始化、密码策略和限流契约以本文说明为准。

Rust API 会在启动时提供编译期生成的 OpenAPI 文档。交互式 Swagger UI 地址为 `/docs/`，OpenAPI JSON 地址为 `/openapi.json`。这些文档由 handler 上的 `#[utoipa::path]` 注解和共享 DTO 的 schema derive 生成；本文档保留补充说明、业务约束和运行行为细节。

## 基本约定

- 基础地址：`http://localhost:8000`
- API 前缀：`/api`
- MCP 端点：`/mcp`，这是 Streamable HTTP MCP 协议端点，不属于 REST API 前缀，也不出现在 OpenAPI schema 中
- 文档中的“需要认证”分两类：

  - 浏览器前端：登录成功后由后端设置 `HttpOnly`、`SameSite=Lax` 的 `ps_session` Cookie，之后同源 `/api/*` 请求自动携带 Cookie。
  - 外部脚本/API 客户端：使用设置页创建的访问令牌，并通过请求头传递：

  ```http
  Authorization: Bearer <access_token>
  ```

  不要把登录令牌或访问令牌放入 `access_token`、`at` 等 URL 查询参数。

### 数据库选择

所有依赖索引库的检索接口都支持可选查询参数 `db`，其值对应 `data/index/` 下的数据库文件名或文件名去掉 `.sqlite` 的形式。

规则：

- 如果 `data/index/` 下只有一个 `.sqlite` 文件，则可省略 `db`
- 如果有多个数据库而未提供 `db`，返回 `400`
- 指定的数据库不存在时返回 `404`

示例：

- `?db=utd24.sqlite`
- `?db=utd24`

### 分页

大多数列表接口使用 offset 分页：

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `limit` | 通常为 `50` 或 `100` | 页大小 |
| `offset` | `0` | 偏移量 |

`/api/articles` 额外支持游标分页：

- `cursor` 格式：`{date}|{article_id}`
- `include_total=false` 时不会计算总数，响应中的 `page.total` 可能为 `null`

`article_id` 与 `journal_id` 在 JSON 响应中以十进制字符串返回，避免浏览器端丢失 64-bit 整数精度。请求参数与路径参数仍使用对应的十进制 ID 文本。

### 缓存

后端会对以下路径添加缓存头：

- `/api/articles*`
- `/api/meta*`

无认证凭据时缓存策略为：

```http
Cache-Control: public, max-age=300, stale-while-revalidate=600
```

如果请求携带 `Authorization` 或 `ps_session` Cookie，响应会使用：

```http
Cache-Control: private, no-store
```

### 跨源浏览器访问

默认前端通过 Next.js rewrite 使用同源 `/api/*`。如果设置 `NEXT_PUBLIC_API_URL` 让浏览器跨源直连后端，管理员必须在运行时配置 `cors_allowed_origins` 中列出允许的 Origin；不要使用 `*` 搭配 Cookie credentials。

### Streamable HTTP MCP

Rust API 进程直接提供 `POST/GET/DELETE /mcp`，用于 MCP Streamable HTTP 传输。该端点复用现有认证：

- `Authorization: Bearer <access_token>`
- 或同源浏览器请求中的 `ps_session` Cookie

`/mcp` 当前暴露的工具包括：

- 元数据：`list_databases`、`list_areas`、`list_years`、`list_journal_options`、`list_sources`
- 期刊：`list_journals`、`get_journal`
- 文章：`search_articles`、`get_article`
- 每周更新：`get_weekly_updates`
- 收藏：`list_folders`、`add_favorite`、`remove_favorite`

所有工具返回 text content，内容为 JSON 字符串。只读工具直接复用索引库读取逻辑；收藏工具使用当前认证用户的 user id，不能访问其他用户的文件夹。

安全配置：

- `mcp_allowed_hosts` 默认只允许 `localhost`、`127.0.0.1` 和 `::1`。通过公网域名、局域网 IP 或反向代理访问 `/mcp` 时，必须显式加入对应 `Host` 或 `host:port`。
- `mcp_allowed_origins` 默认空，表示不校验浏览器 `Origin`。只有浏览器跨源直连 MCP 时才需要配置允许的 Origin。

## 检索与展示接口

除健康检查和公告列表外，本节接口都需要认证。

### 健康检查

| 方法 | 路径 | 认证 | 说明 |
| --- | --- | --- | --- |
| `GET` | `/api/health` | 否 | 返回 `{"status":"ok"}` |

### 元数据接口

| 方法 | 路径 | 认证 | 说明 |
| --- | --- | --- | --- |
| `GET` | `/api/meta/databases` | 是 | 列出 `data/index/` 下可用数据库文件 |
| `GET` | `/api/meta/areas` | 是 | 返回领域及数量 |
| `GET` | `/api/meta/journals` | 是 | 返回期刊选项列表 |
| `GET` | `/api/meta/sources` | 是 | 返回 CSV 中声明的 `source` 值统计 |
| `GET` | `/api/years` | 是 | 返回年份、issue 数与期刊数 |

### 期刊接口

#### `GET /api/journals`

支持参数：

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `db` | string | 数据库名 |
| `area` | string | 领域过滤 |
| `library_id` | string | 数据源过滤；当前值通常为 `scholarly` 或 `cnki` |
| `available` | bool | 是否可用 |
| `has_articles` | bool | 是否存在文章 |
| `year` | int | 只保留该年有 issue 的期刊 |
| `scimago_min` | float | Scimago 下限 |
| `scimago_max` | float | Scimago 上限 |
| `sort` | string | 支持 `journal_id`、`title`、`issn`、`eissn`、`scimago_rank`、`available`、`has_articles` |
| `limit` | int | 默认 `50`，最大 `200` |
| `offset` | int | 默认 `0` |

默认排序：`scimago_rank:desc`

#### `GET /api/journals/{journal_id}`

返回单个期刊详情，包含平台期刊 ID 与 CSV 元数据：

- `platform_journal_id`
- `source_csv`
- `area`
- `csv_title`
- `csv_issn`
- `csv_library`，当前保存 CSV 的 `source` 值

### Issue 接口

#### `GET /api/issues`

支持参数：

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `db` | string | 数据库名 |
| `journal_id` | string/int | 期刊 ID |
| `year` | int | 年份 |
| `is_valid_issue` | bool | 是否有效期次 |
| `suppressed` | bool | 是否抑制 |
| `embargoed` | bool | 是否 embargo |
| `within_subscription` | bool | 是否在订阅范围内 |
| `sort` | string | 支持 `issue_id`、`publication_year`、`title`、`date`、`volume`、`number` |
| `limit` | int | 默认 `50`，最大 `200` |
| `offset` | int | 默认 `0` |

默认排序：`publication_year:desc`

#### `GET /api/issues/{issue_id}`

返回单个 issue 详情。

### 文章接口

#### `GET /api/articles`

支持参数：

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `db` | string | 数据库名 |
| `journal_id` | string/int，可重复 | 多个期刊过滤 |
| `issue_id` | int | issue 过滤 |
| `year` | int | issue 年份过滤 |
| `area` | string，可重复 | 多个领域过滤 |
| `in_press` | bool | 是否 in-press |
| `open_access` | bool | 是否开放获取 |
| `suppressed` | bool | 是否抑制 |
| `within_library_holdings` | bool | 历史字段，当前新抓取数据通常为空 |
| `date_from` | string | 起始日期 |
| `date_to` | string | 截止日期 |
| `doi` | string | DOI 精确过滤 |
| `pmid` | string | PMID 精确过滤 |
| `q` | string | FTS5 查询 |
| `sort` | string | 当前只支持 `date` |
| `limit` | int | 默认 `50`，最大 `200` |
| `offset` | int | 默认 `0` |
| `cursor` | string | 文章游标分页 |
| `include_total` | bool | 是否统计总数 |

补充说明：

- 当 `article_listing` 已准备好时，查询优先走物化辅助表
- 查询参数 `q` 会作为 SQLite FTS5 `MATCH` 表达式传入；中文分词由 `article_search` 的 tokenizer 处理，不做拼音查询展开
- 默认排序：`date:desc`

#### `GET /api/articles/{article_id}`

返回单篇文章详情。

#### `GET /api/articles/{article_id}/access`

返回单篇文章的访问能力，供前端区分“详情/摘要页”和“真正全文”。

响应示例：

```json
{
  "detail": {
    "available": true,
    "label": "查看摘要/详情",
    "provider": "detail_url",
    "url": "https://oversea.cnki.net/openlink/detail?...",
    "requires_login": false,
    "message": null
  },
  "fulltext": {
    "available": false,
    "label": "获取全文",
    "provider": "zjlib_cnki",
    "url": null,
    "requires_login": true,
    "message": "需要先在设置中完成浙江图书馆扫码登录"
  }
}
```

行为：

- 非 CNKI 文章如果有安全的 `full_text_file`，`fulltext.available = true`
- 文章有 `permalink` 或 DOI 时，`detail.available = true`
- CNKI 文章只有当前用户的浙江图书馆 CNKI 会话为 `active` 时才暴露全文能力
- 响应不包含浙江图书馆 token、cookie 值或其他用户凭据

#### `GET /api/articles/{article_id}/fulltext`

执行全文动作。行为由 `/access` 返回的 provider 决定，兼容旧链接：

- 非 CNKI 且存在安全 `full_text_file` 时重定向到该 URL
- CNKI 且当前用户有 active 浙江图书馆 CNKI 会话时，后端按题名搜索、逐条校验题名/作者/期刊完全匹配后返回 PDF
- CNKI 未登录时仍回退到详情页，前端应优先使用 `/access` 判断是否显示“获取全文”
- 无全文 URL 时可按旧逻辑回退到 `permalink` 或 DOI 详情页

CNKI 精确匹配失败时返回受控错误，不会下载候选列表中的错误 PDF。

### 每周更新与公告

| 方法 | 路径 | 认证 | 说明 |
| --- | --- | --- | --- |
| `GET` | `/api/weekly-updates` | 是 | 基于 `data/push_state/*.changes.json` 聚合最近新增文章 |
| `GET` | `/api/announcements` | 否 | 返回当前启用的系统公告，按优先级和时间排序 |

`GET /api/weekly-updates` 不需要查询参数。接口只聚合变更清单中的 `notifiable_article_ids`，并按数据库和期刊分组；响应中的 `window_start` 与 `window_end` 由变更清单时间戳推导。

## 认证与用户接口

所有 `/api/auth/*` 中除注册、登录、邀请码需求检查外，其他均需要认证。

### 注册与登录

| 方法 | 路径 | 认证 | 说明 |
| --- | --- | --- | --- |
| `POST` | `/api/auth/register` | 否 | 使用有效邀请码注册普通账号；永远不会创建管理员 |
| `POST` | `/api/auth/login` | 否 | 登录并签发访问令牌 |
| `GET` | `/api/auth/invite-required` | 否 | 返回邀请码要求和本机管理员初始化状态 |

`POST /api/auth/register` 请求体：

```json
{
  "username": "alice",
  "password": "secret-password",
  "invite_code": "required-code"
}
```

约束：

- 用户名：`3..32` 位，仅允许字母、数字、下划线
- 密码：新注册密码至少 12 个 Unicode 字符
- 邀请码：公开注册始终必填；注册用户始终为非管理员

空安装时，`GET /api/auth/invite-required` 返回：

```json
{
  "required": true,
  "bootstrap_required": true
}
```

管理员通过本机 `admin bootstrap` 创建后，`bootstrap_required` 变为 `false`，`required` 仍保持 `true`。API 不提供远程首管理员创建接口。

`POST /api/auth/login` 请求体：

```json
{
  "username": "alice",
  "password": "secret-password"
}
```

响应包含：

- `user`
- `expires_at`

登录响应不会返回原始登录令牌；浏览器会通过 `Set-Cookie: ps_session=...` 保存 HttpOnly 会话 Cookie。

登录和注册使用进程内有界限流：同一规范化用户名 5 分钟最多 5 次失败尝试；每个 API 进程另有登录每分钟 100 次、注册每分钟 25 次的全局桶。超过限制时返回 `429` 和数值型 `Retry-After`，响应正文统一为 `Too many authentication attempts; try again later`，不会表明用户名是否存在。进程重启会清空这些计数，多副本和公网部署还应在反向代理层增加共享限流。

### 当前用户与改密

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/api/auth/me` | 返回当前用户资料 |
| `POST` | `/api/auth/change-password` | 修改当前用户密码 |
| `POST` | `/api/auth/logout` | 撤销当前登录令牌 |

`POST /api/auth/change-password` 请求体：

```json
{
  "old_password": "old",
  "new_password": "new-secret-password"
}
```

新密码至少 12 个字符。该规则只应用于注册、改密和管理员重置；升级前已经存在的较短密码哈希仍可登录。

### 访问令牌

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `POST` | `/api/auth/tokens` | 创建额外访问令牌 |
| `GET` | `/api/auth/tokens` | 列出当前用户有效令牌 |
| `DELETE` | `/api/auth/tokens/{token_id}` | 撤销指定令牌 |

`POST /api/auth/tokens` 请求体：

```json
{
  "name": "script",
  "ttl": 604800
}
```

后端会把 TTL 约束到 `3600 .. 31536000` 秒。

### 用户自助邀请码

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `POST` | `/api/auth/invite-code` | 为当前用户生成一个一次性邀请码 |
| `GET` | `/api/auth/invite-code` | 查看当前用户已经生成的邀请码 |

每个普通用户最多生成一个邀请码。

## 浙江图书馆 CNKI 会话接口

所有 `/api/cnki/*` 均需要认证，且只读写当前用户自己的浙江图书馆 CNKI 会话。接口响应只返回安全状态信息，不返回 token 或 cookie 值。

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/api/cnki/session` | 查看当前用户 CNKI 会话状态 |
| `POST` | `/api/cnki/login/start` | 启动浙江图书馆扫码登录并返回二维码 payload |
| `POST` | `/api/cnki/login/poll` | 轮询扫码登录结果并保存 active 会话 |
| `DELETE` | `/api/cnki/session` | 清除当前用户 CNKI 会话 |

`GET /api/cnki/session` 响应字段：

- `configured`
- `status`：常见值为 `empty`、`waiting_scan`、`active`、`expired`
- `has_bff_user_token`
- `expires_at`
- `seconds_remaining`
- `cookie_names`
- `updated_at`
- `last_used_at`

`POST /api/cnki/login/start` 响应包含：

- `uuid`
- `status`
- `qr_code`，可能是图片 URL、data URI 或二维码内容文本
- `session`

`POST /api/cnki/login/poll` 请求体：

```json
{
  "timeout_seconds": 15,
  "interval_seconds": 1.5
}
```

成功后会把当前用户的会话状态更新为 `active`。

## 收藏夹接口

所有 `/api/favorites/*` 均需要认证。浏览器下载导出文件时使用 `ps_session` Cookie；外部客户端使用 `Authorization: Bearer <access_token>`。

### 文件夹

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/api/favorites/folders` | 列出当前用户文件夹 |
| `POST` | `/api/favorites/folders` | 创建文件夹 |
| `PUT` | `/api/favorites/folders/{folder_id}` | 重命名文件夹 |
| `DELETE` | `/api/favorites/folders/{folder_id}` | 删除文件夹 |
| `GET` | `/api/favorites/tracking` | 获取当前追踪文件夹 |
| `PUT` | `/api/favorites/tracking` | 指定追踪文件夹 |

创建文件夹请求体：

```json
{
  "name": "Reading",
  "is_tracking": false
}
```

约束：

- 名称长度 `1..100`
- 同一用户下文件夹名称唯一

### 文件夹内文章

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/api/favorites/folders/{folder_id}/articles` | 列出文件夹内文章 |
| `GET` | `/api/favorites/folders/{folder_id}/count` | 返回文章数量 |
| `POST` | `/api/favorites/folders/{folder_id}/articles` | 添加单篇文章 |
| `DELETE` | `/api/favorites/folders/{folder_id}/articles/{article_id}` | 移除文章 |
| `POST` | `/api/favorites/folders/{folder_id}/articles/bulk` | 批量添加文章 |
| `POST` | `/api/favorites/folders/{folder_id}/articles/bulk-remove` | 批量移除文章 |
| `POST` | `/api/favorites/folders/{folder_id}/articles/bulk-move` | 批量移动文章到另一个文件夹 |
| `GET` | `/api/favorites/folders/{folder_id}/export` | 导出引用文件 |

`GET /api/favorites/folders/{folder_id}/articles` 支持：

- `limit`：默认 `100`，最大 `500`
- `offset`：默认 `0`

导出接口支持：

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `format` | `bibtex` | 可选 `bibtex`、`ris`、`endnote` |

导出接口会返回文件下载响应，文件名取自文件夹名并自动清洗。

### 收藏状态查询

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/api/favorites/check` | 检查单篇文章收藏在哪些文件夹 |
| `POST` | `/api/favorites/check/batch` | 批量检查收藏状态 |

## 追踪与通知设置接口

所有 `/api/tracking/*` 需要认证。

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `POST` | `/api/tracking/push-weekly` | 启动当前用户的手动每周推送后台任务 |
| `GET` | `/api/tracking/push-weekly/status` | 查询当前用户手动每周推送任务状态 |
| `GET` | `/api/tracking/status` | 返回追踪状态摘要 |
| `GET` | `/api/tracking/notification-settings` | 获取当前用户通知设置 |
| `PUT` | `/api/tracking/notification-settings` | 更新当前用户通知设置 |

### `POST /api/tracking/push-weekly`

说明：

- 数据来源不是实时日期扫描，而是 `data/push_state/*.changes.json`
- 接口会立即返回后台任务状态；如果已有任务运行，则返回现有任务
- AI 选择是推送前置条件；未配置关键词/方向、AI 配置不可用或 AI 失败时会跳过推送
- `delivery_method = "folder"` 时写入追踪文件夹
- `delivery_method = "pushplus"` 时发送 PushPlus；若 `sync_to_tracking_folder = true`，还会同步写入追踪文件夹

### `PUT /api/tracking/notification-settings`

请求体：

```json
{
  "keywords": ["earnings management"],
  "directions": ["capital markets"],
  "selected_databases": ["utd24.sqlite"],
  "delivery_method": "pushplus",
  "pushplus_token": "token",
  "pushplus_template": "markdown",
  "pushplus_topic": "",
  "pushplus_channel": "wechat",
  "sync_to_tracking_folder": false,
  "ai_base_url": "https://api.siliconflow.cn/v1",
  "ai_api_key": "sk-...",
  "ai_model": "deepseek-ai/DeepSeek-V3",
  "ai_system_prompt": "",
  "ai_backup_base_url": "",
  "ai_backup_api_key": "",
  "ai_backup_model": "",
  "ai_backup_system_prompt": "",
  "ai_retry_attempts": 3,
  "enabled": true
}
```

约束：

- `delivery_method` 当前只允许 `folder` 或 `pushplus`
- 当 `delivery_method = "pushplus"` 时，`pushplus_token` 必填
- `selected_databases` 为空表示全部数据库；传入值必须对应 `data/index/` 下已有 `.sqlite` 文件
- 当 `delivery_method = "pushplus"` 且 `sync_to_tracking_folder = true` 时，必须已经设置追踪文件夹

## 管理员接口

所有 `/api/admin/*` 需要管理员权限。

### 用户与邀请码

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/api/admin/users` | 列出全部用户及统计 |
| `PUT` | `/api/admin/users/{user_id}/admin` | 授予或撤销管理员 |
| `POST` | `/api/admin/users/{user_id}/reset-password` | 管理员重置密码 |
| `DELETE` | `/api/admin/users/{user_id}` | 删除用户 |
| `GET` | `/api/admin/invite-codes` | 列出全部邀请码 |
| `POST` | `/api/admin/invite-codes` | 生成管理员邀请码 |
| `DELETE` | `/api/admin/invite-codes/{code_id}` | 删除未使用邀请码 |

### 统计、运行时配置、定时任务与公告

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/api/admin/stats` | 返回系统综合统计 |
| `GET` | `/api/admin/runtime-settings` | 列出外部元数据服务运行配置及来源 |
| `PUT` | `/api/admin/runtime-settings` | 更新外部元数据服务运行配置 |
| `GET` | `/api/admin/scheduled-tasks` | 列出定时任务 |
| `POST` | `/api/admin/scheduled-tasks` | 创建定时任务 |
| `PUT` | `/api/admin/scheduled-tasks/{task_id}` | 更新定时任务 |
| `DELETE` | `/api/admin/scheduled-tasks/{task_id}` | 删除定时任务 |
| `GET` | `/api/admin/scheduler/status` | 返回调度游标、worker 心跳与最近运行元数据 |
| `GET` | `/api/admin/announcements` | 列出全部公告 |
| `POST` | `/api/admin/announcements` | 创建公告 |
| `PUT` | `/api/admin/announcements/{announcement_id}` | 更新公告 |
| `DELETE` | `/api/admin/announcements/{announcement_id}` | 删除公告 |

### 运行时配置请求体

当前运行时配置用于 Crossref / OpenAlex / Semantic Scholar 抓取链路，以及 API CORS、MCP 与 Cookie 安全策略。配置保存在 `data/auth.sqlite` 的 `runtime_settings` 表中，API、索引器和调度任务启动时直接读取数据库值。

`GET /api/admin/runtime-settings` 返回每个配置项的：

- `field`：API 请求体字段名
- `label`
- `description`
- `input_type`
- `is_secret`
- `value`
- `source`：`database` 或 `default`
- `updated_at`

`PUT /api/admin/runtime-settings` 请求体：

```json
{
  "values": {
    "openalex_api_key_pool": "key1,key2",
    "semantic_scholar_api_key_pool": "s2-key",
    "crossref_mailto_pool": "admin@example.com",
    "cors_allowed_origins": "https://app.example",
    "mcp_allowed_hosts": "paper.example,paper.example:443",
    "mcp_allowed_origins": "https://app.example",
    "secure_cookies": "true"
  }
}
```

当前允许的字段：

| 字段 | 说明 |
| --- | --- |
| `openalex_api_key_pool` | OpenAlex API key 池 |
| `semantic_scholar_api_key_pool` | Semantic Scholar API key 池 |
| `crossref_mailto_pool` | Crossref 联系邮箱池 |
| `cors_allowed_origins` | 允许跨源浏览器访问的 Origin 列表 |
| `mcp_allowed_hosts` | HTTP MCP `Host` 白名单 |
| `mcp_allowed_origins` | HTTP MCP 浏览器 `Origin` 白名单 |
| `secure_cookies` | `ps_session` Cookie 是否带 `Secure` 标记 |

未知字段会返回 `400`。清空某个值会把该配置保存为空字符串，列表接口仍会把该项显示为 `database` 来源。

### 定时任务请求体

创建：

```json
{
  "name": "nightly notify",
  "job": {
    "kind": "notify",
    "database": "utd24.sqlite",
    "max_candidates": 200
  },
  "cron": "0 8 * * *",
  "timezone": "Asia/Shanghai",
  "timeout_seconds": 3600,
  "coalesce": true,
  "enabled": true
}
```

创建时 `timezone`、`timeout_seconds`、`coalesce` 和 `enabled` 可以省略，默认分别为 `UTC`、`3600`、`true` 和 `true`。更新时所有字段都可以省略。

`job` 是带 `kind` 标签的类型化对象，只支持以下组合：

| `kind` | 可选字段 | 行为 |
| --- | --- | --- |
| `index` | `metadata_file`、`notify`、`push` | 运行索引更新，并可在成功后顺序执行通知或文件夹推送 |
| `notify` | `database`、`max_candidates` | 执行外部通知 |
| `push` | `database`、`max_candidates` | 执行追踪文件夹推送 |

`metadata_file` 只能是 `.csv` 文件名，`database` 只能是 `.sqlite` 文件名；二者都不能包含目录、空格或 shell 特殊字符。`max_candidates` 必须是 `1-1000` 的整数。未知 `kind`、未知字段和旧版 `command` 字段会被拒绝。

补充说明：

- `cron` 使用标准五段 crontab，并在 `timezone` 指定的 IANA 时区中计算；非法时区和不在 `1-86400` 秒范围内的超时会返回 `400`
- 夏令时跳过的本地分钟不会执行；回拨后重复出现且都匹配 cron 的本地分钟会对应两个不同的 UTC 执行槽
- Docker 默认运行 `worker --project-root /app --interval-seconds 30`。worker 使用持久化检查游标补齐轮询间隔内的执行槽，最长回看 24 小时；`coalesce = true` 时只保留最近一个错过的槽
- 每个 `(task_id, scheduled_for)` 只有一条运行记录，同一任务同时最多有一个 `claimed` 或 `running` 实例，因此多个 worker 不会重复执行同一槽
- 每个任务的 `timeout_seconds` 覆盖完整类型化 job 链路；一个任务失败或超时不会阻止同轮的其他任务继续执行
- 立即执行和 dry-run 可通过 `scheduler run-once TASK_ID` 与 `scheduler dry-run-once TASK_ID` 从运维终端触发
- worker 只会直接启动固定的 `index`、`notify`、`push` 可执行文件，并把已验证字段转换为独立 argv；不会调用 shell
- 从旧版 `command` 列迁移的任务会保留 `legacy_command` 供审阅，但强制禁用，必须先替换为 `job` 才能启用或执行
- 没有单独的“立即执行”管理 API

`GET /api/admin/scheduler/status` 返回：

- `last_checked_at`：最近完成的持久化检查时间，尚未运行时为 `null`
- `workers`：worker 标识、启动时间、最近心跳和按 90 秒窗口计算的 `is_healthy`
- `recent_runs`：最近 20 条运行的任务、计划时间、状态和认领/开始/完成时间

运行状态包括 `pending`、`claimed`、`running`、`success`、`failed`、`timed_out`、`error` 和 `unknown`。接口不会返回进程 stdout、stderr 或内部错误摘要。

### 公告请求体

创建：

```json
{
  "title": "系统维护",
  "message": "今晚 22:00 进行维护",
  "priority": "high",
  "enabled": true
}
```

更新体可部分提交，`priority` 允许：

- `high`
- `normal`
- `low`

## 常见错误码

| 状态码 | 场景 |
| --- | --- |
| `400` | 参数非法、用户名或密码格式错误、cron 无效、缺少邀请码等 |
| `401` | 未认证或 Bearer 格式错误 |
| `403` | 非管理员访问管理员接口 |
| `404` | 目标记录或数据库不存在 |
| `409` | 用户名冲突、文件夹重名等唯一约束冲突 |
| `429` | 登录或注册超过进程内认证限流；响应含 `Retry-After` |
