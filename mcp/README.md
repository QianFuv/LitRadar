# Paper Scanner MCP

`mcp/` 是 Paper Scanner 的 MCP Server 子项目，使用 `@modelcontextprotocol/sdk` 通过 stdio 暴露一组面向 Paper Scanner API 的工具。

## 当前能力

该 MCP 服务当前主要覆盖以下几类能力：

- 索引数据库枚举
- 领域与年份元数据查询
- 期刊列表查询
- 文章搜索与单篇文章读取
- 每周更新聚合读取
- 已登录用户的收藏夹读取与收藏增删

它不是对后端所有 `/api/*` 路由的完整映射，而是当前项目里一组常用的 MCP 工具封装。

## 运行前提

- Node.js `>= 20`
- 可访问的 Paper Scanner 后端 API

默认 API 地址：

- `http://localhost:8000`

## 安装与构建

在 `mcp/` 目录下执行：

```bash
npm install
npm run build
```

构建产物输出到：

- `dist/`

入口文件：

- `dist/index.js`

## 启动方式

该服务使用 **stdio transport**，适合由支持 MCP 的客户端托管启动。

直接启动示例：

```bash
node dist/index.js
```

如果已经全局或本地以 bin 方式调用，也可以使用：

```bash
paper-scanner-mcp
```

## 环境变量

### 必填或常用变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `PAPER_SCANNER_API_URL` | `http://localhost:8000` | Paper Scanner 后端根地址 |
| `PAPER_SCANNER_API_TOKEN` | 空 | 访问需要认证工具时使用的 Bearer Token |
| `PAPER_SCANNER_DB` | 空 | 默认数据库名，会自动附加到支持 `db` 的请求上 |

说明：

- `PAPER_SCANNER_API_TOKEN` 只在需要认证的工具里使用
- 如果调用认证工具但未设置 `PAPER_SCANNER_API_TOKEN`，服务会直接返回错误
- `PAPER_SCANNER_DB` 可用于省去反复手动传 `db`

## 当前注册的工具

### 元数据工具

| 工具名 | 说明 |
| --- | --- |
| `list_databases` | 列出可用 SQLite 数据库 |
| `list_areas` | 列出指定数据库的研究领域 |
| `list_years` | 列出指定数据库的发表年份统计 |

### 期刊工具

| 工具名 | 说明 |
| --- | --- |
| `list_journals` | 列出指定数据库中的期刊 |

### 文章工具

| 工具名 | 说明 |
| --- | --- |
| `search_articles` | 搜索文章，支持领域、日期、开放获取、关键词、年份等过滤 |
| `get_article` | 按文章 ID 读取单篇文章 |

### 每周更新工具

| 工具名 | 说明 |
| --- | --- |
| `get_weekly_updates` | 获取全库每周更新摘要 |

### 收藏工具

以下工具需要认证：

| 工具名 | 说明 |
| --- | --- |
| `list_folders` | 列出当前用户收藏夹 |
| `add_favorite` | 向指定文件夹添加文章 |
| `remove_favorite` | 从指定文件夹移除文章 |

## 参数约定

### 数据库参数

支持数据库参数的工具通常接收：

- `db`

如果未传：

- 优先使用 `PAPER_SCANNER_DB`
- 再交给后端按默认规则解析

### 收藏相关参数

收藏工具通常同时需要：

- `folder_id`
- `article_id`
- `db_name`

其中 `db_name` 可省略，省略时会退回到：

- `PAPER_SCANNER_DB`
- 若仍为空，则传空字符串给后端

## 响应格式

当前实现会把后端返回值统一封装为 MCP 文本内容：

```json
{
  "content": [
    {
      "type": "text",
      "text": "{...json...}"
    }
  ]
}
```

也就是说，工具结果本质上是格式化后的 JSON 文本。

## 与后端的对应关系

当前 MCP 工具大致映射到这些后端路由：

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

## 目录结构

```text
mcp/
├── src/
│   ├── index.ts          MCP 服务入口
│   ├── client.ts         API 客户端与通用响应封装
│   └── tools/            各类工具注册
├── dist/                 TypeScript 编译输出
├── package.json
└── tsconfig.json
```

## 示例配置

不同 MCP 客户端的配置格式会略有差异，但核心思路一致：通过 stdio 启动 `node dist/index.js`，并注入环境变量。

示例：

```json
{
  "command": "node",
  "args": ["D:/QianFuv/Paper-Scanner/mcp/dist/index.js"],
  "env": {
    "PAPER_SCANNER_API_URL": "http://localhost:8000",
    "PAPER_SCANNER_API_TOKEN": "your-access-token",
    "PAPER_SCANNER_DB": "utd24.sqlite"
  }
}
```

## 开发注意事项

- 新增工具时，先在 `src/tools/` 下增加对应文件或扩展现有文件
- 工具输入模式使用 `zod`
- 当前客户端统一通过 `PaperScannerClient` 访问后端
- 认证工具务必显式设置 `auth: true`
