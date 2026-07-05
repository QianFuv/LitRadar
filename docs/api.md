# API 参考

本文档以当前 Rust API 实现为准，覆盖检索接口、认证接口、收藏/追踪接口以及管理员接口。后端部署与正常运行入口已经切换到 Rust；公开路径、请求体、响应体、认证 Cookie/Bearer 行为和 SQLite 数据契约保持不变。

当前 API 没有内置自动生成的 Swagger 或 OpenAPI 页面，接口信息以本文档为准。

## 基本约定

- 基础地址：`http://localhost:8000`
- API 前缀：`/api`
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

默认前端通过 Next.js rewrite 使用同源 `/api/*`。如果设置 `NEXT_PUBLIC_API_URL` 让浏览器跨源直连后端，后端必须通过逗号分隔的 `API_CORS_ALLOWED_ORIGINS` 显式列出允许的 Origin；不要使用 `*` 搭配 Cookie credentials。

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
- 当 `article_search` 使用了 `simple` 分词器且查询词不含 CJK 字符时，会自动改用 `simple_query(...)`
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
| `POST` | `/api/auth/register` | 否 | 注册账号；首个用户不需要邀请码且自动成为管理员 |
| `POST` | `/api/auth/login` | 否 | 登录并签发访问令牌 |
| `GET` | `/api/auth/invite-required` | 否 | 返回当前是否需要邀请码注册 |

`POST /api/auth/register` 请求体：

```json
{
  "username": "alice",
  "password": "secret123",
  "invite_code": "optional-code"
}
```

约束：

- 用户名：`3..32` 位，仅允许字母、数字、下划线
- 密码：至少 6 位

`POST /api/auth/login` 请求体：

```json
{
  "username": "alice",
  "password": "secret123"
}
```

响应包含：

- `user`
- `expires_at`

登录响应不会返回原始登录令牌；浏览器会通过 `Set-Cookie: ps_session=...` 保存 HttpOnly 会话 Cookie。

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
  "new_password": "new-secret"
}
```

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
| `GET` | `/api/admin/announcements` | 列出全部公告 |
| `POST` | `/api/admin/announcements` | 创建公告 |
| `PUT` | `/api/admin/announcements/{announcement_id}` | 更新公告 |
| `DELETE` | `/api/admin/announcements/{announcement_id}` | 删除公告 |

### 运行时配置请求体

当前运行时配置用于 Crossref / OpenAlex / Semantic Scholar / CNKI 抓取链路。配置保存在 `data/auth.sqlite` 的 `runtime_settings` 表中，API、索引器和调度任务启动时会把数据库值应用到进程环境变量；数据库已有值会覆盖同名宿主环境变量。

`GET /api/admin/runtime-settings` 返回每个配置项的：

- `field`：API 请求体字段名
- `key`：实际环境变量名
- `label`
- `description`
- `input_type`
- `is_secret`
- `value`
- `source`：`database`、`environment` 或 `default`
- `updated_at`

`PUT /api/admin/runtime-settings` 请求体：

```json
{
  "values": {
    "openalex_api_key_pool": "key1,key2",
    "semantic_scholar_api_key_pool": "s2-key",
    "crossref_mailto_pool": "admin@example.com",
    "proxy_pool": "socks5://127.0.0.1:1080"
  }
}
```

当前允许的字段：

| 字段 | 环境变量 | 说明 |
| --- | --- | --- |
| `openalex_api_key_pool` | `OPENALEX_API_KEY_POOL` | OpenAlex API key 池 |
| `semantic_scholar_api_key_pool` | `SEMANTIC_SCHOLAR_API_KEY_POOL` | Semantic Scholar API key 池 |
| `crossref_mailto_pool` | `CROSSREF_MAILTO_POOL` | Crossref 联系邮箱池 |
| `proxy_pool` | `PROXY_POOL` | scholarly 与 CNKI 请求代理池 |

未知字段会返回 `400`。清空某个值会把该配置保存为空字符串；下次应用运行配置时会移除同名进程环境变量，列表接口仍会把该项显示为 `database` 来源。

### 定时任务请求体

创建：

```json
{
  "name": "nightly notify",
  "command": "notify --db utd24.sqlite --changes-file /app/data/push_state/utd24.changes.json --no-dry-run",
  "cron": "0 8 * * *",
  "enabled": true
}
```

更新时四个字段都可以省略。

补充说明：

- `cron` 使用标准五段 crontab
- Docker 默认运行 `ps-cli worker shadow`，用于持续加载并校验定时任务配置；不会自动执行 shell 命令
- 立即执行和 dry-run 可通过 `ps-cli scheduler run-once TASK_ID` 与 `ps-cli scheduler dry-run-once TASK_ID` 从运维终端触发
- 执行模式仍按 shell 命令处理，并在执行前应用 `runtime_settings` 数据库配置
- 没有单独的“立即执行”管理 API

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
