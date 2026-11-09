# LitRadar

LitRadar 是面向学术期刊的自托管检索与订阅平台。它从 Crossref、OpenAlex、Semantic Scholar 和 CNKI 获取元数据，构建本地 SQLite 全文索引，通过 Web 界面提供检索、收藏、每周更新、文献追踪和期刊征稿追踪。

## 能力概览

- 多源索引：英文期刊使用 Scholarly 流程，中文期刊使用 CNKI 流程。
- 本地检索：基于 FTS5，支持中文短语检索。
- 用户工作区：账号、邀请码、访问令牌、收藏夹和引用导出。
- 文献追踪：通过 OpenAI 兼容模型筛选文章，再发送 PushPlus 通知或写入追踪文件夹。
- 征稿追踪：查看期刊征稿信息，并通过命令导入或刷新来源记录。
- 管理与接入：运行配置、定时任务、服务状态、公告，以及 REST API、OpenAPI 和 Streamable HTTP MCP。

## 运行组成

Go 服务端入口为 `apps/server`，`litradar serve` 提供 REST、Swagger/OpenAPI、MCP 和持久化调度。Wails v3 客户端位于 `apps/client`，与服务端共用根 Go 模块，共享唯一 Vite 前端 `apps/client/frontend`。Web 部署通过 Caddy 将同一 Origin 的接口请求转发给服务端、页面转发给 Linux amd64 Wails Server Build；服务端根路径返回 404。只有服务端访问数据与部署密钥。Simple 分词器静态链接进服务端，Obscura、Poppler 和期刊元数据仍随服务端部署。见[系统架构](docs/architecture/README.md)。

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

Compose 运行 `server`、`client-web`、`gateway` 三个服务，只有网关向 loopback 发布 8000 端口。服务端镜像为 `ghcr.io/qianfuv/litradar:latest`，客户端为 `ghcr.io/qianfuv/litradar-client-web:latest`。从源码构建时先在根目录执行 `pnpm install --frozen-lockfile && pnpm build`，再执行 `docker compose up -d --build`。镜像自带官方期刊目录，首次运行会准备 `data/meta/`，后续升级保留自定义文件；完整规则见 [Meta 与持久卷](docs/operations/docker.md#meta-bundle-与持久卷)。

### 3. 初始化首个管理员

公开注册不能创建首个管理员。在交互式 Bash 中读取密码后，通过 stdin 传给命令：

```bash
IFS= read -r -s -p 'Admin password: ' ADMIN_PASSWORD
printf '\n'
printf '%s\n' "$ADMIN_PASSWORD" |
  docker compose run --rm -T server admin bootstrap \
    --username admin \
    --password-stdin
unset ADMIN_PASSWORD
```

密码至少需要 12 个 Unicode 字符，不把实际值写入参数、Compose 文件或命令历史。命令只在用户表为空时成功。

### 4. 准备索引

CNKI 元数据索引不需要 Scholarly API key，可先运行：

```bash
docker compose run --rm server index \
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

项目使用 Go 1.27.3（启用 CGO）、Node.js 26.11.1 和 pnpm 12.16.1；构建静态 Simple 还需要与 Go 目标匹配的 C/C++14 编译器、CMake、curl 和 tar，Windows 另需 MinGW 和 Ninja。环境准备、原生分词器和开发命令见[开发指南](docs/guides/development.md)，前端内部结构见[前端说明](apps/client/frontend/README.md)。

开发时在根目录运行 `pnpm dev`，由 `scripts/dev.mjs` 一起启动 Vite（8000）和 Go 服务端（8001）。根目录维护唯一 `pnpm-lock.yaml`。

## 客户端

Web、Windows x64 和 Android arm64-v8a 共用同一 Vite 前端。首次打开原生客户端先填写服务端 HTTPS URL，再用账号密码或 OIDC（服务端启用时）登录；两种方式都签发名称为 `device:` 前缀、有效期 90 天的个人令牌。可在访问令牌列表撤销设备，原生退出会撤销当前令牌。见[客户端指南](docs/guides/clients.md)。

发布提供 Web Linux amd64 二进制及 OCI 镜像、未签名的 Windows x64 安装器，以及配置发布签名后提供的 Android arm64 APK。Wails v3 官方将 [Android 支持标为实验性](https://wails.io/blog/wails-v3-released)。不发布 iOS、macOS 或 Linux 桌面客户端。

## 版本发布

普通推送只运行质量检查；发布时修改根目录 `VERSION`（例如 `0.1.0` → `0.1.1`），提交并推送到 `main`。检查通过后自动发布 GitHub Release 二进制包及 `ghcr.io/qianfuv/litradar:v0.1.1`、`latest` 镜像，无需手动打标签。服务端二进制包支持 Linux amd64 和 Windows x64，仅附带服务端与原生辅助程序；容器镜像只提供 `linux/amd64`。Actions 的 `CI` 负责检查，也可手动运行并填写当前 `VERSION` 重试发布；`Release` 是由 `CI` 调用的可复用发布流程。版本规则、安装依赖和失败重试见[版本发布说明](docs/operations/releases.md)。

## 文档

从[文档中心](docs/README.md)选择阅读路径：

- 开发：[系统架构](docs/architecture/README.md)、[开发指南](docs/guides/development.md)、[测试系统](docs/guides/testing.md)。
- 运维：[Docker 部署](docs/operations/docker.md)、[备份与恢复](docs/operations/backup.md)、[日志运维](docs/operations/logging.md)。
- 使用与接入：[通知与追踪](docs/guides/notifications.md)、[API 参考](docs/reference/api.md)、[CLI 参考](docs/reference/cli.md)。

<a id="license"></a>

## 许可证

本项目使用 [MIT License](LICENSE)。
