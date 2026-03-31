# Paper Scanner MCP

[Paper Scanner](https://github.com/QianFuv/Paper-Scanner) 的 MCP Server —— 学术期刊索引与追踪系统。通过 stdio 提供文章搜索、元数据查询、每周更新和收藏管理等工具。

## 快速开始

### 使用 npx（无需安装）

```json
{
  "command": "npx",
  "args": ["-y", "@qianfuv/paper-scanner-mcp"],
  "env": {
    "PAPER_SCANNER_API_URL": "https://your-api-host",
    "PAPER_SCANNER_API_TOKEN": "your-access-token"
  }
}
```

### 全局安装

```bash
npm install -g @qianfuv/paper-scanner-mcp
```

```json
{
  "command": "paper-scanner-mcp",
  "env": {
    "PAPER_SCANNER_API_URL": "https://your-api-host",
    "PAPER_SCANNER_API_TOKEN": "your-access-token"
  }
}
```

## 环境变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `PAPER_SCANNER_API_URL` | `http://localhost:8000` | Paper Scanner 后端地址 |
| `PAPER_SCANNER_API_TOKEN` | — | **必填。** 用于 API 认证的 Bearer Token |
| `PAPER_SCANNER_DB` | — | 默认数据库名，自动附加到支持 `db` 参数的请求 |

所有 API 端点均需要认证，Token 会作为 `Bearer` 头随每个请求发送。可在 Paper Scanner 设置页面中生成。

## 工具列表

### 元数据

| 工具名 | 说明 |
| --- | --- |
| `list_databases` | 列出可用的 SQLite 数据库 |
| `list_areas` | 列出指定数据库的研究领域 |
| `list_years` | 列出指定数据库的发表年份统计 |

### 期刊

| 工具名 | 说明 |
| --- | --- |
| `list_journals` | 列出指定数据库中的期刊 |

### 文章

| 工具名 | 说明 |
| --- | --- |
| `search_articles` | 搜索文章，支持领域、日期、期刊、关键词、开放获取等过滤 |
| `get_article` | 按 ID 获取单篇文章 |

### 每周更新

| 工具名 | 说明 |
| --- | --- |
| `get_weekly_updates` | 获取全库每周新增文章摘要 |

### 收藏

| 工具名 | 说明 |
| --- | --- |
| `list_folders` | 列出当前用户的收藏夹 |
| `add_favorite` | 向指定文件夹添加文章 |
| `remove_favorite` | 从指定文件夹移除文章 |

## API 路由映射

| MCP 工具 | 后端路由 |
| --- | --- |
| `list_databases` | `GET /api/meta/databases` |
| `list_areas` | `GET /api/meta/areas` |
| `list_years` | `GET /api/years` |
| `list_journals` | `GET /api/journals` |
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
