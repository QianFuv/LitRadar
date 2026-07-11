# 系统架构

本文档说明 Paper Scanner 当前的组件边界、进程模型、数据流和持久化结构。命令参数见 [CLI 参考](reference/cli.md)，具体部署步骤见 [Docker 部署](operations/docker.md)。

## 总览

```text
data/meta/*.csv
        |
        v
      index ----> data/index/*.sqlite
        |                 |
        |                 +----> api <----> Next.js app
        |                           |
        +----> *.changes.json       +----> Streamable HTTP MCP
                    |
                    +----> notify ----> PushPlus
                    |
                    +----> push ------> tracking folder

data/auth.sqlite
  users / tokens / folders / settings / jobs / runtime config
        ^                         |
        |                         v
       api <------------------- worker
```

系统没有 Python 运行时路径。后端服务、索引、调度、通知和维护命令均由 Rust workspace 提供。

## 运行进程

| 进程        | 生命周期   | 职责                                                                |
| ----------- | ---------- | ------------------------------------------------------------------- |
| `api`       | 常驻       | REST API、Swagger/OpenAPI、MCP、认证、检索和管理接口                |
| `worker`    | 常驻       | 扫描持久化调度槽、认领任务、启动类型化 `index`/`notify`/`push` 作业 |
| `app`       | 常驻       | Next.js Web 界面，通过同源 rewrite 调用 API                         |
| `index`     | 按需或调度 | 读取期刊 CSV、请求上游、写索引库和变更清单                          |
| `notify`    | 按需或调度 | AI 选择后发送 PushPlus                                              |
| `push`      | 按需或调度 | AI 选择后写入用户追踪文件夹                                         |
| `scheduler` | 按需       | 校验任务或立即执行一个已保存的类型化任务                            |
| `admin`     | 按需、本机 | 初始化管理员、维护密文、备份和离线恢复                              |

Docker Compose 默认运行 `api`、`worker` 和 `app`。其余命令复用后端镜像按需启动。

## Rust workspace

| Crate          | 责任                                                          |
| -------------- | ------------------------------------------------------------- |
| `ps-api`       | Axum 路由、OpenAPI、MCP、请求日志和异步边界                   |
| `ps-auth`      | 密码、会话、访问令牌和认证服务                                |
| `ps-cli`       | 所有独立 Rust 命令的参数解析与编排                            |
| `ps-domain`    | 跨 crate 的领域结构和响应类型                                 |
| `ps-index`     | 索引 schema、转换、写入、统计和变更清单                       |
| `ps-recommend` | 候选排序、AI 配置解析、消息内容和投递状态                     |
| `ps-sources`   | Crossref、OpenAlex、Semantic Scholar、CNKI 和浙江图书馆客户端 |
| `ps-storage`   | SQLite 迁移、查询、业务存储、密文和备份恢复                   |
| `ps-worker`    | 调度认领、AI/PushPlus 传输和投递编排                          |

依赖方向以领域结构和存储接口为中心；二进制入口保持很薄，业务逻辑放在 crate 中测试。

## 持久化边界

### 元数据输入

`data/meta/*.csv` 定义需要索引的期刊。核心列包括：

| 列          | 含义                  |
| ----------- | --------------------- |
| `source`    | `scholarly` 或 `cnki` |
| `title`     | 期刊标题              |
| `issn`      | 首选 ISSN             |
| `id`        | 稳定的上游/项目标识   |
| `area`      | 项目领域标签          |
| `all_issns` | 可选 ISSN 候选列表    |

### 索引数据库

每个 CSV 对应 `data/index/<csv_stem>.sqlite`。索引库包含期刊、期次、文章、FTS5、物化筛选表和索引运行统计。详见[数据库参考](reference/database.md)。

### 认证与业务数据库

`data/auth.sqlite` 保存：

- 用户、会话和访问令牌
- CNKI 用户会话
- 收藏夹和收藏文章
- 通知/追踪设置
- 全局运行配置
- 类型化定时任务、运行槽和心跳
- 系统公告

该库不保存 32 字节部署密钥；受保护的集成凭据以认证密文写入数据库，密钥通过文件单独提供。

### 外部状态文件

| 路径                                | 所有者                                              |
| ----------------------------------- | --------------------------------------------------- |
| `data/push_state/<db>.changes.json` | `index --update` 生成；每周更新、通知和追踪共同读取 |
| `data/push_state/<db>.json`         | `notify` 和手动 PushPlus 投递状态                   |
| `data/folder_push_state/<db>.json`  | `push` 的追踪文件夹投递状态                         |

变更清单是新文章分发的输入，不是可从文章日期实时重建的视图。

## 主要数据流

### Scholarly 索引

1. `index` 读取 `source=scholarly` 的 CSV 行。
2. Crossref 按 ISSN 提供文章主列表。
3. Crossref 对所有 ISSN 返回 404 时，OpenAlex 解析 source 并作为文章列表 fallback。
4. OpenAlex 按 DOI 增强元数据；Semantic Scholar 增强 OA、PDF 和缺失摘要。
5. `ps-index` 写入关系表、`article_listing` 和 `article_search`。
6. `--update` 生成变更清单。

具体请求和字段优先级见 [Scholarly 数据源](reference/sources/scholarly.md)。

### CNKI 索引和全文

CNKI 元数据索引使用公开 overseas 页面和接口；按用户全文获取使用独立的浙江图书馆会话。索引器不把权限控制链接当作可直接访问的全文。详见 [CNKI 数据源](reference/sources/cnki.md)。

### 检索

1. 前端通过 `app/lib/api/` 调用 `/api/*`。
2. API 根据可选 `db` 选择一个 `data/index/*.sqlite`。
3. 列表查询在 `listing_state=ready` 且物化表非空时使用 `article_listing`，否则回退到关系表联查。
4. `q` 使用 `article_search MATCH`。
5. API 把 64 位文章和期刊 ID 序列化为十进制字符串。

### 认证

浏览器登录后使用 `HttpOnly` 的 `ps_session` Cookie。外部客户端使用用户创建的 Bearer 访问令牌。首个管理员只能通过本机 `admin bootstrap` 创建；公开注册始终要求邀请码。

### 通知和追踪

1. 从变更清单或状态快照差异得到候选文章。
2. 按用户的数据库、关键词和方向偏好缩小候选。
3. 使用用户级 OpenAI 兼容凭据做主备 AI 选择。
4. 根据 `delivery_method` 发送 PushPlus 或写入追踪文件夹。
5. 成功副作用完成后更新 `delivery_dedupe`。

完整行为见[通知与追踪](guides/notifications.md)。

## 配置层次

系统使用多种明确分离的配置来源：

- CLI 参数：进程路径、端口、并发和一次运行的覆盖
- `runtime_settings`：外部元数据 key 池、CORS、MCP 和 Cookie 策略
- `notification_settings`：每个用户的 AI、PushPlus 和追踪偏好
- 前端环境变量：浏览器 API 地址和服务端 rewrite 目标
- 部署密钥文件：只用于认证和解密数据库中的秘密值

来源、默认值和优先级见[运行配置](reference/configuration.md)。

## 数据库迁移

所有正式后端入口在业务访问前执行版本化 SQLite 迁移：

1. 解析路径和参数。
2. 检查 `PRAGMA user_version`。
3. 在独立 `BEGIN IMMEDIATE` 事务中逐版本迁移。
4. 在同一事务末尾更新版本。
5. 遇到未来版本或失败立即退出。

API 在迁移成功后才绑定端口，worker 在迁移成功后才进入循环。普通查询仓库不负责 DDL。

## 同步工作与异步服务

SQLite、PBKDF2、阻塞 HTTP、文件系统和手动推送编排是同步工作。API 通过 `ApiState` 的有界阻塞执行器把这些工作送入 Tokio blocking pool，避免在路由或 MCP future 中直接阻塞运行时。worker 和 CLI 本身按同步作业模型执行。

API 共享的 `blocking executor` 有 8 个 permit。除此之外，手动周推在每个 API 进程内按 `auth.sqlite` 路径设置 1 个 admission slot：同一用户重复启动会复用当前 running job，不同用户竞争同一 storage instance 时立即收到 `503`，因此最多 1 个 manual API job 等待或占用共享 permit。该边界不排队、不持久化，也不是 `cross-process` 锁；CLI、scheduler、worker 或其他 API 进程的投递并不受它协调。

## 部署边界

默认 Compose 只把前端和 API 发布到宿主机 loopback，三个常驻容器均为非 root、只读根文件系统，并丢弃 Linux capabilities。公网部署必须增加 TLS 反向代理和共享限流，不能把默认端口直接改为所有网卡。详见 [Docker 部署](operations/docker.md)和[安全说明](operations/security.md)。
