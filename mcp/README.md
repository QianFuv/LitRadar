# Paper Scanner MCP

[Paper Scanner](https://github.com/QianFuv/Paper-Scanner) 的 MCP 工具集用于学术期刊索引与追踪系统，提供文章搜索、元数据查询、每周更新和收藏管理等工具。

当前推荐的 HTTP 部署路径是 Rust API 进程内置的 Streamable HTTP MCP 端点：`/mcp`。本目录的 Node 包保留为 stdio 兼容路径，供只支持 stdio MCP 的客户端继续使用；不需要单独运行 Node HTTP MCP 服务。

## Rust HTTP MCP

启动 Rust API 后，HTTP MCP 地址为：

```text
http://localhost:8000/mcp
```

该端点复用现有 API 认证：

- 外部 MCP 客户端使用 `Authorization: Bearer <access_token>`
- 浏览器同源调用可使用现有 `ps_session` Cookie

非 loopback 域名或反向代理访问时，需要在 API 进程上配置 `MCP_ALLOWED_HOSTS`，例如：

```bash
MCP_ALLOWED_HOSTS=paper.example,paper.example:443
```

浏览器跨源直连 HTTP MCP 时，再配置 `MCP_ALLOWED_ORIGINS`。普通服务器端 MCP 客户端通常不需要 Origin 配置。

## 快速开始

以下命令使用 Node stdio MCP 兼容包。HTTP MCP 客户端应直接连接 Rust API 的 `/mcp`，并携带上述认证凭据。

### 使用 claude code

```
claude mcp add paper-scanner --scope user -e PAPER_SCANNER_API_URL=<api-url> -e PAPER_SCANNER_API_TOKEN=<api-token> -- cmd /c npx -y @qianfuv/paper-scanner-mcp
```

### 使用 codex

```
codex mcp add paper-scanner --env PAPER_SCANNER_API_URL=<api-url> --env PAPER_SCANNER_API_TOKEN=<api-token> -- npx -y @qianfuv/paper-scanner-mcp
```

## 环境变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `PAPER_SCANNER_API_URL` | `http://localhost:8000` | Paper Scanner 后端地址 |
| `PAPER_SCANNER_API_TOKEN` | — | **必填。** 用于 API 认证的 Bearer Token |
| `PAPER_SCANNER_DB` | — | 默认数据库名，自动附加到支持 `db` 参数的请求；单个工具显式传入 `db` 时会覆盖该默认值 |

MCP 工具调用的后端端点需要认证，Token 会作为 `Bearer` 头随每个请求发送。可在 Paper Scanner 设置页面中生成。

## 工具列表

### 元数据

| 工具名 | 说明 |
| --- | --- |
| `list_databases` | 列出可用的 SQLite 数据库 |
| `list_areas` | 列出指定数据库的研究领域 |
| `list_years` | 列出指定数据库的发表年份统计 |
| `list_journal_options` | 列出指定数据库的期刊筛选选项 |
| `list_sources` | 列出指定数据库的元数据来源 |

### 期刊

| 工具名 | 说明 |
| --- | --- |
| `list_journals` | 列出指定数据库中的期刊，支持领域、来源、可用性、年份、Scimago 与排序过滤 |
| `get_journal` | 按 ID 获取单个期刊 |

### 文章

| 工具名 | 说明 |
| --- | --- |
| `search_articles` | 搜索文章，支持领域、日期、期刊、issue、关键词、开放获取、DOI、PMID、排序与分页等过滤 |
| `get_article` | 按 ID 获取单篇文章 |

### 每周更新

| 工具名 | 说明 |
| --- | --- |
| `get_weekly_updates` | 获取全库每周新增文章摘要 |

每周更新来自后端 `data/push_state/*.changes.json` 变更清单，响应窗口由清单时间戳推导，不是按当前日期实时扫描索引库。

### 收藏

| 工具名 | 说明 |
| --- | --- |
| `list_folders` | 列出当前用户的收藏夹 |
| `add_favorite` | 向指定文件夹添加文章 |
| `remove_favorite` | 从指定文件夹移除文章 |

## Node stdio API 路由映射

以下映射仅描述本目录 Node stdio 兼容包如何调用 Rust API 的 REST 路由。Rust HTTP MCP 端点 `/mcp` 在 API 进程内直接执行对应工具逻辑。

| MCP 工具 | 后端路由 |
| --- | --- |
| `list_databases` | `GET /api/meta/databases` |
| `list_areas` | `GET /api/meta/areas` |
| `list_years` | `GET /api/years` |
| `list_journal_options` | `GET /api/meta/journals` |
| `list_sources` | `GET /api/meta/sources` |
| `list_journals` | `GET /api/journals` |
| `get_journal` | `GET /api/journals/{journal_id}` |
| `search_articles` | `GET /api/articles` |
| `get_article` | `GET /api/articles/{article_id}` |
| `get_weekly_updates` | `GET /api/weekly-updates` |
| `list_folders` | `GET /api/favorites/folders` |
| `add_favorite` | `POST /api/favorites/folders/{folder_id}/articles` |
| `remove_favorite` | `DELETE /api/favorites/folders/{folder_id}/articles/{article_id}` |

## 开发

```bash
cd mcp
npm install
npm run build
```

源码在 `src/`，编译输出在 `dist/`。工具注册在 `src/tools/`，API 客户端在 `src/client.ts`。

## 许可

MIT
