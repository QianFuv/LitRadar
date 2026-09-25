# LitRadar

LitRadar 是面向学术期刊的自托管检索与订阅平台。它从 Crossref、OpenAlex、Semantic Scholar 和 CNKI 获取元数据，构建本地 SQLite 全文索引，通过 Web 界面提供检索、收藏、每周更新、文献追踪和期刊征稿追踪。

## 能力概览

- 多源索引：英文期刊使用 Scholarly 流程，中文期刊使用 CNKI 流程。
- 本地检索：基于 FTS5；新建 v9 索引使用随项目提供的 `simple 0` 分词器，支持中文短语检索。
- 用户工作区：账号、邀请码、访问令牌、收藏夹和引用导出。
- 文献追踪：通过 OpenAI 兼容模型筛选文章，再发送 PushPlus 通知或写入追踪文件夹。
- 征稿追踪：查看期刊征稿信息，并通过命令导入或刷新来源记录。
- 管理与接入：运行配置、定时任务、服务状态、公告，以及 REST API、OpenAPI 和 Streamable HTTP MCP。

## 运行组成

应用入口是唯一的 `litradar` 可执行文件。`litradar serve` 同时承载静态 Web、REST、Swagger/OpenAPI、MCP 和持久化调度；任务需要隔离时，由它启动短生命周期的同名子命令。Next.js 前端构建后由 Rust 提供静态资源，部署时不需要单独运行 Node.js 服务。模块边界和数据流见[系统架构](docs/architecture.md)。

## Docker 快速开始

以下步骤用于首次本机部署，在仓库根目录的 **Bash** 中执行，需要 Docker Engine 或 Docker Desktop、Docker Compose、OpenSSL 和 curl。Windows 用户可使用 WSL Bash；不要直接把 Bash 的续行和输入语法粘贴到 PowerShell。已有数据的升级或 HTTPS 接入请使用 [Docker 部署](docs/operations/docker.md)中的对应流程。

### 1. 准备数据目录和部署密钥

仅在尚无部署密钥时生成新文件；已有数据库必须继续使用与它匹配的密钥。

```bash
mkdir -p data secrets
if [ ! -e secrets/litradar.key ]; then
  (umask 077; openssl rand -out secrets/litradar.key 32)
fi
```

Linux 原生 Docker Engine 需要让容器账号 `10001:10001` 读写数据目录、读取密钥。下面命令针对本次专用部署目录：

```bash
sudo chown -R 10001:10001 data
sudo chown 10001:10001 secrets/litradar.key
sudo chmod 600 secrets/litradar.key
```

Docker Desktop 通常由虚拟化层处理挂载权限，不照搬上述 `chown`。密钥必须恰好为 32 个原始字节，与数据库备份分开保管。已有明文集成凭据的部署先按[安全说明](docs/operations/security.md)迁移。

### 2. 启动服务

```bash
docker compose pull
docker compose up -d --remove-orphans
docker compose ps
```

Compose 运行一个 `litradar` 服务，本机示例使用 `ghcr.io/qianfuv/litradar:latest`。从当前源码构建时，改用 `docker compose up -d --build --remove-orphans`。镜像自带官方期刊目录，首次运行会准备 `data/meta/`，后续升级保留自定义文件；完整规则见 [Meta 与持久卷](docs/operations/docker.md#meta-bundle-与持久卷)。

### 3. 初始化首个管理员

公开注册不能创建首个管理员。在交互式 Bash 中读取密码后，通过 stdin 传给命令：

```bash
IFS= read -r -s -p 'Admin password: ' ADMIN_PASSWORD
printf '\n'
printf '%s\n' "$ADMIN_PASSWORD" |
  docker compose run --rm -T litradar admin bootstrap \
    --username admin \
    --password-stdin
unset ADMIN_PASSWORD
```

密码至少需要 12 个 Unicode 字符，不把实际值写入参数、Compose 文件或命令历史。命令只在用户表为空时成功。

### 4. 准备索引

CNKI 元数据索引不需要 Scholarly API key，可先运行：

```bash
docker compose run --rm litradar index \
  --secret-key-file /run/secrets/litradar_key \
  --file chinese_journals.csv \
  --update
```

`--update` 执行增量更新并发布每周更新所需的变更清单；首次运行或没有可复用的成功边界时会完整扫描。遇到验证码时，按 [CNKI 数据源说明](docs/reference/sources/cnki.md)配置验证码服务。

索引 `english_journals.csv` 或 `ccf_computer_journals.csv` 前，先登录管理后台，在运行配置中填写 Crossref 联系邮箱、OpenAlex 和 Semantic Scholar API key。全量核对、中断恢复与并发参数见 [CLI 参考](docs/reference/cli.md)，配额与默认值见[运行配置参考](docs/reference/configuration.md)。

<a id="5-访问服务"></a>

### 5. 验证并访问

```bash
curl --fail http://localhost:8000/health/ready
curl --fail --output /dev/null http://localhost:8000/
```

两个请求都应成功。打开 `http://localhost:8000/` 登录，选择已完成索引的数据库并检索一篇已知文章。接口入口为 `/api`，Swagger UI 为 `/docs/`，OpenAPI JSON 为 `/openapi.json`，MCP 为 `/mcp`；MCP 客户端需要会话或访问令牌。

## 本地开发

项目使用 Rust 1.96、Node.js 24 和 pnpm 10.32.0。环境准备、原生分词器和开发命令见[开发指南](docs/guides/development.md)，前端内部结构见[前端说明](app/README.md)。

## 文档

从[文档中心](docs/README.md)选择阅读路径：

- 开发：[系统架构](docs/architecture.md)、[开发指南](docs/guides/development.md)、[测试系统](docs/testing.md)。
- 运维：[Docker 部署](docs/operations/docker.md)、[备份与恢复](docs/operations/backup.md)、[日志运维](docs/operations/logging.md)。
- 使用与接入：[通知与追踪](docs/guides/notifications.md)、[API 参考](docs/reference/api.md)、[CLI 参考](docs/reference/cli.md)。

<a id="license"></a>

## 许可证

本项目使用 [MIT License](LICENSE)。
