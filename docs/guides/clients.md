# 客户端与登录

`apps/server` 是 Go API 与任务服务端。`apps/client` 是独立 Wails v3 模块，所有客户端共享 `apps/client/frontend` 中唯一的 Vite 前端与根 `pnpm-lock.yaml`。

## 发布平台

| 平台 | 产物 | 说明 |
| --- | --- | --- |
| Web | Linux amd64 Wails Server Build 二进制与 client-web OCI 镜像 | Caddy 提供同源 UI/API 入口 |
| Windows | x64 安装器 | 未签名；连接独立部署的服务端 |
| Android | arm64-v8a APK | 仅发布签名构建；Wails 官方支持为实验性 |

不发布 iOS、macOS 或 Linux 桌面客户端。[Wails 官方发布说明](https://wails.io/blog/wails-v3-released)说明 Android/mobile 的实验性状态。

## 原生端设置与认证

首次启动 Windows 或 Android 客户端时填写服务端 HTTPS URL，例如 `https://papers.example.org`。URL 指向 Go 服务端 API 可达的公开入口；Web 浏览器使用当前 Origin 的 API，不需要此设置。

使用本地账号密码登录时，客户端向 `POST /api/auth/native/login` 提交 `username`、`password`、`device_name`。响应包含 `token`、`expires_at` 和用户；不会写入浏览器 Cookie。服务端签发独立、90 天有效的个人访问令牌，名称以 `device:` 开头。后续请求使用 `Authorization: Bearer <token>`。服务端只保存 token hash，客户端不能把令牌放入 URL、日志或分享内容。

在浏览器或原生客户端的访问令牌列表中找到设备令牌即可撤销；原生退出登录撤销当前个人令牌。设备过期或被撤销后重新登录。此流程不覆盖浏览器的 Cookie 会话。

仅 OIDC 的用户先通过浏览器登录并创建访问令牌，再粘贴到原生客户端。该粘贴令牌使用创建时选择的有效期；90 天固定 TTL 只适用于原生密码登录签发的设备令牌。暂不支持原生 OIDC deep link。

## Web 部署

浏览器经 Caddy 访问页面与 API。`/api`、`/mcp`、`/docs` 及子路径、`/openapi.json`、`/health/*` 交给服务端，其他页面由 Web host 的 SPA 提供。`/wails` 和 `/wails/*` 直接返回 404：Wails Server Build 的 runtime 端点没有认证，Web host 不注册服务，不能将 runtime 端点作为公开 API。

Web host 只在内部网络监听 8080，不挂载业务数据或部署密钥。公开域名、Cookie、可信代理和 MCP 设置见 [Docker 部署](../operations/docker.md)；构建任务、Windows 未签名状态及 Android 签名 secrets 见[发布说明](../operations/releases.md)。
