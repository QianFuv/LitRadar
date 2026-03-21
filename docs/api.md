# API 参考

本文档以当前 FastAPI 代码为准，覆盖公开检索接口、认证接口、收藏/追踪接口以及管理员接口。

## 基本约定

- 基础地址：`http://localhost:8000`
- API 前缀：`/api`
- 文档中的“需要认证”指需要在请求头中携带：

  ```http
  Authorization: Bearer <access_token>
  ```

### 数据库选择

所有依赖索引库的公开检索接口都支持可选查询参数 `db`，其值对应 `data/index/` 下的数据库文件名或文件名去掉 `.sqlite` 的形式。

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

### 缓存

后端会对以下路径添加缓存头：

- `/api/articles*`
- `/api/meta*`

缓存策略为：

```http
Cache-Control: public, max-age=300, stale-while-revalidate=600
```

## 公开检索接口

### 健康检查

| 方法 | 路径 | 认证 | 说明 |
| --- | --- | --- | --- |
| `GET` | `/api/health` | 否 | 返回 `{"status":"ok"}` |

### 元数据接口

| 方法 | 路径 | 认证 | 说明 |
| --- | --- | --- | --- |
| `GET` | `/api/meta/databases` | 否 | 列出 `data/index/` 下可用数据库文件 |
| `GET` | `/api/meta/areas` | 否 | 返回领域及数量 |
| `GET` | `/api/meta/journals` | 否 | 返回期刊选项列表 |
| `GET` | `/api/meta/libraries` | 否 | 返回 CSV 中声明的 `library` 值统计 |
| `GET` | `/api/years` | 否 | 返回年份、issue 数与期刊数 |

### 期刊接口

#### `GET /api/journals`

支持参数：

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `db` | string | 数据库名 |
| `area` | string | 领域过滤 |
| `library_id` | string | `journals.library_id` 过滤 |
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

返回单个期刊详情，包含 CSV 元数据：

- `source_csv`
- `area`
- `csv_title`
- `csv_issn`
- `csv_library`

### Issue 接口

#### `GET /api/issues`

支持参数：

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `db` | string | 数据库名 |
| `journal_id` | int | 期刊 ID |
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
| `journal_id` | int，可重复 | 多个期刊过滤 |
| `issue_id` | int | issue 过滤 |
| `year` | int | issue 年份过滤 |
| `area` | string，可重复 | 多个领域过滤 |
| `in_press` | bool | 是否 in-press |
| `open_access` | bool | 是否开放获取 |
| `suppressed` | bool | 是否抑制 |
| `within_library_holdings` | bool | 是否馆藏内可访问 |
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

#### `GET /api/articles/{article_id}/fulltext`

重定向到文章全文地址，优先级由代码动态决定，可能落到：

- DOI 链接
- LibKey / full text file
- BrowZine / CQVIP 对应来源页面

### 每周更新与公告

| 方法 | 路径 | 认证 | 说明 |
| --- | --- | --- | --- |
| `GET` | `/api/weekly-updates` | 否 | 基于 `data/push_state/*.changes.json` 聚合最近新增文章 |
| `GET` | `/api/announcements` | 否 | 返回当前启用的系统公告，按优先级和时间排序 |

`GET /api/weekly-updates` 支持参数：

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `window_days` | `7` | 回看天数，范围 `1..31` |

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
- `access_token`
- `expires_at`

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

## 收藏夹接口

所有 `/api/favorites/*` 均需要认证，只有导出接口额外支持 `access_token` 查询参数。

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
| `GET` | `/api/favorites/folders/{folder_id}/export` | 导出引用文件 |

`GET /api/favorites/folders/{folder_id}/articles` 支持：

- `limit`：默认 `100`，最大 `500`
- `offset`：默认 `0`

导出接口支持：

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `format` | `bibtex` | 可选 `bibtex`、`ris`、`endnote` |
| `access_token` | 空 | 可用原始访问令牌代替 Bearer 头 |

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
| `POST` | `/api/tracking/push-weekly` | 将最近每周新增文章推入当前用户追踪文件夹 |
| `GET` | `/api/tracking/status` | 返回追踪状态摘要 |
| `GET` | `/api/tracking/notification-settings` | 获取当前用户通知设置 |
| `PUT` | `/api/tracking/notification-settings` | 更新当前用户通知设置 |

### `POST /api/tracking/push-weekly`

说明：

- 数据来源不是实时日期扫描，而是 `data/push_state/*.changes.json`
- 如果用户配置了关键词或研究方向，会先做 AI 选择
- AI 配置不可用时会回退为“全部推入”

### `PUT /api/tracking/notification-settings`

请求体：

```json
{
  "keywords": ["earnings management"],
  "directions": ["capital markets"],
  "delivery_method": "pushplus",
  "pushplus_token": "token",
  "pushplus_template": "markdown",
  "pushplus_topic": "",
  "pushplus_channel": "wechat",
  "ai_base_url": "https://api.siliconflow.cn/v1",
  "ai_api_key": "sk-...",
  "ai_model": "deepseek-ai/DeepSeek-V3",
  "ai_system_prompt": "",
  "enabled": true
}
```

约束：

- `delivery_method` 当前只允许 `folder` 或 `pushplus`
- 当 `delivery_method = "pushplus"` 时，`pushplus_token` 必填

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

### 统计、定时任务与公告

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/api/admin/stats` | 返回系统综合统计 |
| `GET` | `/api/admin/scheduled-tasks` | 列出定时任务 |
| `POST` | `/api/admin/scheduled-tasks` | 创建定时任务 |
| `PUT` | `/api/admin/scheduled-tasks/{task_id}` | 更新定时任务 |
| `DELETE` | `/api/admin/scheduled-tasks/{task_id}` | 删除定时任务 |
| `GET` | `/api/admin/announcements` | 列出全部公告 |
| `POST` | `/api/admin/announcements` | 创建公告 |
| `PUT` | `/api/admin/announcements/{announcement_id}` | 更新公告 |
| `DELETE` | `/api/admin/announcements/{announcement_id}` | 删除公告 |

### 定时任务请求体

创建：

```json
{
  "name": "nightly notify",
  "command": "uv run notify --db utd24.sqlite",
  "cron": "0 8 * * *",
  "enabled": true
}
```

更新时四个字段都可以省略。

补充说明：

- `cron` 使用标准五段 crontab
- 任务由后端内置 APScheduler 执行
- 执行方式是 `subprocess.run(..., shell=True)`
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
