# LitRadar

LitRadar 是一个面向学术期刊的自托管检索与订阅平台。它从 Crossref、OpenAlex、Semantic Scholar 和 CNKI overseas 获取元数据，构建 SQLite 全文检索库，并通过 Web 界面提供检索、收藏、每周更新、文献追踪和后台管理。

## 能力概览

- 多数据源索引：英文期刊使用 scholarly 流程，中文期刊使用 CNKI 流程
- SQLite 检索：基于 FTS5，可自动加载仓库内置的 `simple` 中文分词扩展
- 用户工作区：账号、邀请码、访问令牌、收藏夹和引用导出
- 文献追踪：OpenAI 兼容模型筛选、PushPlus 通知或追踪文件夹写入
- 管理后台：用户、运行配置、类型化定时任务、服务状态和公告
- 外部接入：REST API、OpenAPI/Swagger UI 和 Streamable HTTP MCP

## 运行组成

LitRadar 只发布一个可执行文件 `litradar`。`litradar serve` 是应用组合根：一个常驻进程同时承载 Web、REST、Swagger/OpenAPI、MCP 和持久化调度。计划任务需要隔离时，由该进程使用当前 `litradar` 可执行文件启动短生命周期的 `index`、`notify` 或 `push` 子进程；这些子进程不是独立服务。

| 组件                 | 职责                                                                 |
| -------------------- | -------------------------------------------------------------------- |
| `crates/litradar/`   | 唯一二进制、命令分发、HTTP/调度生命周期、信号和失败耦合              |
| `litradar serve`     | 唯一常驻应用进程                                                     |
| `litradar <command>` | 管理、索引、投递、手动调度和 OpenAPI 等按需命令                      |
| `app/`               | Next.js 前端源码；构建为静态资源后由同一 Rust 进程提供，不是运行服务 |

完整的模块边界和数据流见[系统架构](docs/architecture.md)。

## Docker 快速开始

前提：

- Docker Engine 或 Docker Desktop
- Docker Compose
- 可生成 32 字节随机文件的 `openssl`

### 1. 准备数据目录和部署密钥

```bash
mkdir -p secrets
openssl rand -out secrets/litradar.key 32
chmod 600 secrets/litradar.key
```

Linux 原生 Docker Engine 还需要让容器内固定账号 `10001:10001` 读写数据目录：

```bash
sudo chown -R 10001:10001 data
```

密钥必须恰好为 32 个原始字节，并与数据库备份分开保管。已有明文集成凭据的部署应先阅读[安全说明](docs/operations/security.md)，不要直接启动。

### 2. 启动服务

Compose 只运行一个名为 `litradar` 的容器，镜像为 `ghcr.io/qianfuv/litradar:latest`。使用已发布镜像：

```bash
docker compose pull
docker compose up -d --remove-orphans
```

需要从当前源码构建时，改用 `docker compose up -d --build --remove-orphans`。

### 3. 初始化首个管理员

公开注册不能创建首个管理员。请从安全输入或密码管理器向 stdin 提供密码：

```bash
printf '%s\n' "$ADMIN_PASSWORD" |
  docker compose run --rm -T litradar admin bootstrap \
    --username admin \
    --password-stdin
```

密码至少需要 12 个 Unicode 字符，不要把实际值写入参数、Compose 文件或命令历史。

### 4. 准备索引

仓库自带 `data/meta/*.csv`。CNKI 元数据索引不需要 scholarly API key，可先验证完整链路：

```bash
docker compose run --rm litradar index \
  --secret-key-file /run/secrets/litradar_key \
  --file chinese_journals.csv \
  --update
```

索引默认使用 `--processes 1 --workers 8 --issue-batch 8`，以控制容器峰值内存。可以显式覆盖这些参数，但更高并发不再保证约 100 MiB 的索引内存目标。`admin`、`index`、`notify`、`push`、`scheduler` 和 `openapi` 是同步短生命周期命令，不会创建 Tokio 工作线程池；只有常驻的 `serve` 使用小型异步运行时。

索引 `english_journals.csv` 或 `ccf_computer_journals.csv` 前，先登录管理后台，在“运行配置”中填写 OpenAlex 和 Semantic Scholar API key。所有命令和参数见 [CLI 参考](docs/reference/cli.md)，配置来源与默认值见[运行配置参考](docs/reference/configuration.md)。

### 5. 访问服务

- Web：`http://localhost:8000/`
- REST API：`http://localhost:8000/api`
- Swagger UI：`http://localhost:8000/docs/`
- OpenAPI JSON：`http://localhost:8000/openapi.json`
- Streamable HTTP MCP：`http://localhost:8000/mcp`

生产发布、反向代理、健康检查和权限要求见 [Docker 部署](docs/operations/docker.md)。

## 本地开发

项目使用 Rust 1.96、Node.js 24 和 pnpm 10.32.0。开发环境、代码生成和检查命令统一记录在[开发指南](docs/guides/development.md)，前端包的内部结构见[前端说明](app/README.md)。

## 文档

从[文档中心](docs/README.md)按目标查找资料：

- 理解系统：[系统架构](docs/architecture.md)
- 参与开发：[开发指南](docs/guides/development.md)
- 部署与运维：[Docker 部署](docs/operations/docker.md)
- 调用接口：[API 参考](docs/reference/api.md)
- 查询命令：[CLI 参考](docs/reference/cli.md)
- 理解存储：[数据库参考](docs/reference/database.md)
- 配置追踪：[通知与追踪](docs/guides/notifications.md)

## License

本项目使用 [MIT License](LICENSE)。
