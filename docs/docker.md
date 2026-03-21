# Docker 部署说明

本文档说明仓库内 Docker 镜像与 Compose 编排的实际行为，并以当前代码为准修正旧文档中的若干过时说法。

## 服务拓扑

根目录 `docker-compose.yml` 当前定义了两个服务：

```text
浏览器
  ├── http://localhost:3000  -> app (Next.js)
  └── http://localhost:8000  -> api (FastAPI)
```

注意：

- 根 Compose 文件里 **同时暴露了 3000 和 8000**
- 旧文档中“只有 3000 对外暴露”的说法已不再成立

## 根 Compose 文件

### `api` 服务

- 构建上下文：仓库根目录
- Dockerfile：根目录 `Dockerfile`
- 镜像名：`ghcr.io/qianfuv/paper-scanner-api:latest`
- 端口：`8000:8000`
- 卷挂载：`./data:/app/data`
- 环境变量：`API_HOST=0.0.0.0`

### `app` 服务

- 构建上下文：`./app`
- Dockerfile：`app/Dockerfile`
- 镜像名：`ghcr.io/qianfuv/paper-scanner-app:latest`
- 端口：`3000:3000`
- 环境变量：`HOSTNAME=0.0.0.0`
- 依赖：`depends_on: [api]`

## 镜像构建细节

### 后端镜像

后端镜像分两阶段构建：

1. `build` 阶段
   - 基础镜像：`python:3.12-slim-trixie`
   - 使用 `uv sync --frozen --no-dev`
   - 复制 `scripts/`

2. 运行阶段
   - 基础镜像：`python:3.12-slim-trixie`
   - 复制 `.venv/`、`scripts/`
   - 复制 `libs/simple-linux/`
   - 复制 `data/meta/`
   - 默认启动命令：`uv run api`

运行时默认环境变量：

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `API_HOST` | `0.0.0.0` | Uvicorn 监听地址 |
| `SIMPLE_TOKENIZER_PATH` | `/app/libs/simple-linux/libsimple-linux-ubuntu-latest/libsimple` | `simple` 分词扩展路径 |

### 前端镜像

前端镜像同样采用多阶段构建：

1. `deps`
   - 基础镜像：`node:20-alpine`
   - 使用 `pnpm install --frozen-lockfile`

2. `build`
   - 默认构建参数：`INTERNAL_API_URL=http://api:8000`
   - 执行 `pnpm build`

3. 运行阶段
   - 复制 `.next/standalone`
   - 复制 `.next/static`
   - 启动命令：`node server.js`

说明：

- 根 Compose 没有显式设置 `INTERNAL_API_URL`，但 `app/Dockerfile` 的构建参数默认值已经是 `http://api:8000`
- 因此前端镜像在 Docker 网络内会把 `/api/*` 请求转发到 `api` 服务，而不是容器内的 `localhost`

## 快速启动

### 本地构建

```bash
docker compose build
docker compose up -d
```

### 直接拉取 GHCR 镜像

```bash
docker compose pull
docker compose up -d
```

访问地址：

- 前端：`http://localhost:3000`
- API：`http://localhost:8000/api`

## 首次初始化建议

首次部署后通常还需要建立索引数据库：

```bash
docker compose run --rm api uv run index
```

如果只想处理某个 CSV：

```bash
docker compose run --rm api uv run index --file utd24.csv
```

## 数据与挂载目录

### `data/`

`api` 服务会把宿主机 `./data` 挂载到容器 `/app/data`。运行中涉及的主要文件包括：

- `data/meta/*.csv`：输入的期刊元数据 CSV
- `data/index/*.sqlite`：生成的检索数据库
- `data/auth.sqlite`：用户、收藏、通知与后台管理数据库
- `data/push_state/*.json`：通知状态
- `data/push_state/*.changes.json`：变更清单
- `data/folder_push_state/*.json`：追踪文件夹推送状态

根 Compose 当前不再给 `app` 服务挂载额外配置目录。前端认证完全依赖后端 `/api/auth/*` 与运行时环境变量。

## 常用环境变量

### 后端

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `API_HOST` | `127.0.0.1`（本地） / `0.0.0.0`（Docker） | API 监听地址 |
| `SIMPLE_TOKENIZER_PATH` | 自动探测或镜像内置 | 中文分词扩展路径 |
| `NOTIFY_AI_BASE_URL` | `https://api.siliconflow.cn/v1` | 默认 OpenAI 兼容 API 地址 |
| `NOTIFY_AI_API_KEY` | 空 | 默认 AI Key |
| `NOTIFY_AI_MODEL` | `deepseek-ai/DeepSeek-V3` | 默认模型名 |
| `NOTIFY_AI_SYSTEM_PROMPT` | 空 | 默认系统提示词 |
| `NOTIFY_MAX_CANDIDATES` | `120` | AI 候选上限 |
| `NOTIFY_TEMPERATURE` | `0.2` | AI 温度 |
| `NOTIFY_PUSHPLUS_CHANNEL` | `wechat` | PushPlus 默认渠道 |
| `NOTIFY_PUSHPLUS_TEMPLATE` | `markdown` | PushPlus 默认模板 |
| `NOTIFY_PUSHPLUS_TOPIC` | 空 | PushPlus 默认 topic |
| `NOTIFY_PUSHPLUS_OPTION` | 空 | PushPlus 默认 option |

### 前端

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `HOSTNAME` | 运行时决定 | `next start` / standalone 监听地址 |
| `INTERNAL_API_URL` | `http://api:8000`（Docker 构建默认值） | 构建时用于 `/api/*` rewrite |
| `NEXT_PUBLIC_API_URL` | 空 | 浏览器直接访问的 API 根地址，常用于本地开发 |

## 与测试目录的区别

`test/docker-compose.yml` 与根 Compose 不完全相同：

- 测试 Compose 会显式给前端设置 `INTERNAL_API_URL=http://api:8000`

这些测试资产可作为历史兼容参考，但生产与常规本地部署应以根目录 `docker-compose.yml` 和当前代码行为为准。

## 常见问题

### 1. 前端能打开，但搜索没有数据

排查顺序：

- 检查 `api` 服务是否已启动：`docker compose logs api`
- 检查 `data/index/` 下是否已有 `.sqlite` 文件
- 如无索引库，先执行：`docker compose run --rm api uv run index`

### 2. 中文搜索命中差

优先确认 `simple` 分词扩展是否加载成功：

- Docker 镜像默认已经复制 Linux 版扩展
- 本地运行可通过 `SIMPLE_TOKENIZER_PATH` 手动指定

### 3. 通知或追踪推送没有产生结果

需要同时检查：

- 是否存在最新的 `data/push_state/*.changes.json`
- 用户是否在 `notification_settings` 中启用了对应投递方式
- PushPlus 或 OpenAI 兼容模型配置是否完整
