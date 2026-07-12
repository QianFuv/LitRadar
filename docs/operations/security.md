# 安全说明

本文档记录当前实现的密钥、凭据、认证、限流、网络和容器安全边界。备份恢复的操作顺序见[备份与恢复](backup.md)。

## 部署密钥

后端使用一个 32 字节原始文件认证和解密数据库中的秘密值。它不是口令、十六进制文本或 Base64：

```bash
mkdir -p secrets
openssl rand -out secrets/litradar.key 32
chmod 600 secrets/litradar.key
wc -c secrets/litradar.key
```

`wc` 必须输出 `32`。

密钥必须：

- 不进入 Git、镜像层、Compose YAML、环境变量或 SQLite
- 不出现在日志、命令参数值或普通备份
- 与数据库备份分开存储
- 同时提供给所有需要解密同一 `auth.sqlite` 的进程

`api`、`worker`、`index`、`notify`、`push` 和 `scheduler` 都要求 `--secret-key-file`。`admin secrets` 使用相应 key 参数；`admin bootstrap` 和 `admin backup` 不需要密钥。

## 数据库凭据加密

以下非空字段使用 `litradarenc:v1:` XChaCha20-Poly1305 认证信封：

- `notification_settings.pushplus_token`
- `notification_settings.ai_api_key`
- `notification_settings.ai_backup_api_key`
- `runtime_settings.openalex_api_key_pool`
- `runtime_settings.semantic_scholar_api_key_pool`
- `cnki_sessions.session_json`

每次写入生成随机 24 字节 nonce，并把表、行/配置键和字段名作为关联数据。密文复制到其他用户或字段后无法通过认证。

进程在绑定端口或进入循环前验证现有秘密值。密钥缺失、长度错误、密文损坏、密钥不匹配或残留明文都会使启动失败；错误消息不包含凭据。

当前二进制只接受 `litradarenc:v1:` 信封。改名前的信封格式不会被读取或自动迁移；`admin secrets migrate` 只把明文转换为当前格式。

Crossref 联系邮箱、CORS、MCP 和 Cookie 设置不是秘密字段，以普通运行配置保存。

## API 脱敏与更新语义

通知设置只返回 `has_*` 和固定 `••••` 掩码。管理员运行配置的秘密项返回：

- 空 `value`
- `has_value`
- 空字符串或固定 `masked_value`
- `secret_items`；非秘密项和非池秘密项为空数组

OpenAlex 和 Semantic Scholar 密钥池的每个 `secret_items` 元素包含：

- `masked_value`：正常密钥保留前 5 个字符，其余字符逐个替换为 `*`
- `reference`：只用于单项删除的字段绑定认证密文，不是数据库中持久化的整池密文

长度不超过 5 个字符的异常密钥全部显示为 `*`，不会完整回显。掩码保留星号数量，因此会披露密钥字符总长度；这是为管理员识别密钥而接受的边界。API 不返回完整密钥，也不把持久密文用作显示值或更新值。

`PUT /api/admin/runtime-settings` 的 `values` 保持原有秘密更新语义：

- 字段缺省或空白字符串：保留
- JSON `null`：清除
- 非空字符串：替换

可选的 `secret_pool_updates` 对单个秘密池执行增量操作：

- `add`：按逗号、分号或换行拆分，去除空项并按首次出现顺序去重后追加
- `remove`：提交 `secret_items.reference`，后端解密并精确匹配当前池中的完整值

删除不按前 5 个字符、掩码或列表序号匹配。损坏、跨字段或已经失效的引用返回 `400`，整个事务回滚。后端先解析 `values`，再执行同一字段的增量操作，最后把规范化后的完整池作为一个新的认证密文写入数据库。

前端必须使用单独的清除操作发送 `null`，不能把 `masked_value` 或 `reference` 放进 `values`。不透明引用只应在管理员页面内短暂保存并原样用于 `remove`。

## 首个管理员

远程 API 永远不能创建首个管理员。空库的 `GET /api/auth/invite-required` 返回 `required=true` 和 `bootstrap_required=true`。

管理员只能在能访问 `data/auth.sqlite` 的本机维护环境创建：

```bash
printf '%s\n' "$ADMIN_PASSWORD" |
  admin bootstrap \
    --username admin \
    --password-stdin
```

约束：

- 只接受 stdin，不接受密码值参数
- 用户表必须为空
- `BEGIN IMMEDIATE` 保证并发调用最多一个成功
- 不提升已有用户，也不是密码恢复命令
- stdout/stderr 不输出密码

容器示例见 [Docker 部署](docker.md)。

## 注册、密码和令牌

- 公开注册始终需要未使用的邀请码，只创建普通用户。
- 用户名长度 `3..32`，只允许字母、数字和下划线。
- bootstrap、注册、改密和管理员重置的新密码至少 12 个 Unicode 字符。
- 既有短密码哈希仍可登录，直到下次改密。
- 密码使用 PBKDF2-HMAC-SHA256 hash 和独立 salt。
- 浏览器登录令牌只通过 `HttpOnly`、`SameSite=Lax` 的 `litradar_session` Cookie 传输。
- 用户创建的长期令牌只通过 Bearer 请求头用于外部客户端。
- 令牌不得放入 URL 查询参数。

## 登录和注册限流

每个 API 进程维护固定窗口：

| 桶                          |   限制 |   窗口 |
| --------------------------- | -----: | -----: |
| 规范化用户名，登录/注册共享 |   5 次 | 5 分钟 |
| 全局登录                    | 100 次 | 1 分钟 |
| 全局注册                    |  25 次 | 1 分钟 |

用户名会 trim 并转为 ASCII 小写。成功登录或注册清除对应用户名桶；失败保留到窗口结束。最多跟踪 4096 个用户名，容量满时淘汰最旧项。

超过限制返回 `429`、数值 `Retry-After` 和统一错误，不泄露用户名是否存在。计数仅在单进程内存中；重启清空，多副本或公网部署必须增加代理层共享限流。

## Cookie、CORS 和 MCP

默认 `secure_cookies=false`，适合 loopback HTTP。生产 HTTPS 应先把数据库设置改为 `true`，再用 `api --require-secure-cookies` 作为启动门；不满足时 API 在绑定端口前失败。

生产 Web 静态资源和后端命名空间由同一个 Rust 监听器直接提供，因此浏览器默认同源调用 API，不经过 Next.js 服务或生产 rewrite。本地开发由 Next.js 8000 端口代理内部 Rust 8001；浏览器跨源直连时：

- 在 `cors_allowed_origins` 列出准确 Origin
- credentialed CORS 拒绝 `*` wildcard，避免把任意网站纳入携带 Cookie 的信任边界
- CORS 也拒绝 opaque `null`，避免把不同不透明上下文视为同一个受信 Origin
- 浏览器请求携带 Cookie credentials

MCP 的 `Host` 防护与浏览器 CORS 分开：

- `mcp_allowed_hosts` 默认 `localhost,127.0.0.1,::1`
- 公网域名、局域网 IP 或反向代理 Host 必须显式加入
- `mcp_allowed_origins` 只用于浏览器跨源直连 MCP
- MCP Origin 通常遵循同一准确 HTTP(S) tuple 语法，但为现有 opaque Origin 客户端保留精确字面量 `null`

管理员提交无效 CORS/MCP Origin 时，API 返回 `400` 且整份更新不落库；有效修改在下次 API 启动时生效。旧版本或库外修改留下的无效行会在监听端口绑定前使启动明确失败，不会静默忽略或自动修复，从而避免意外放宽网络策略。

全局配置详见[运行配置参考](../reference/configuration.md)。

### 静态 Web 缓存

生产导出的 Web 文件是公开构建产物，不得包含部署密钥或用户秘密。Rust 按以下边界设置缓存：

- 成功的 `/_next/static/*` 哈希资源使用 `public, max-age=31536000, immutable`，即使请求携带会话 Cookie 也不会变成用户专属内容。
- 页面、导航 payload 和导出的 404 使用 `no-cache`，以便浏览器重新验证版本。
- 受保护 API、携带 Bearer/Cookie 的非静态响应和 `401` 继续使用 `private, no-store`。
- 支持 gzip 的客户端读取镜像内预压缩兄弟文件；不支持的客户端读取原文件。

`/api`、`/mcp`、`/docs` 和 `/openapi.json` 始终由后端路由优先处理，未知路径不会借静态 fallback 读取项目数据或密钥。

## 网络暴露

根 Compose 仅发布：

- `127.0.0.1:8000:8000`

该 Rust 入口同时提供 Web、REST、Swagger/OpenAPI 和 MCP。容器内监听 `0.0.0.0` 只用于 Compose 网络通信。远程访问应经 TLS 反向代理，并同时配置 Secure Cookie、准确 CORS/MCP 白名单和共享限流。不要直接把宿主机端口改为所有网卡。

## 容器边界

`api` 和 `worker` 两个常驻容器使用同一个无后缀镜像：

- 两者都使用 UID/GID `10001:10001`；最终镜像没有 Node.js 运行时
- 根文件系统只读
- `/tmp` 使用 `noexec,nosuid` tmpfs
- 只允许 `/app/data` 持久写入；`/app/web` 保持只读
- 丢弃全部 Linux capabilities
- 启用 `no-new-privileges:true`
- 提供独立健康检查

不要通过 root 容器、开放整个宿主机目录或挂载 Docker socket 解决权限问题。

## 旧明文迁移

普通启动不会自动迁移明文凭据。维护窗口内：

1. 停止 API、worker 和所有可能写 `auth.sqlite` 的 CLI。
2. 创建并验证独立数据库备份。
3. 生成并单独保存部署密钥。
4. 执行迁移。
5. 执行验证。
6. 给所有服务挂载同一密钥后再启动。

```bash
admin secrets migrate \
  --secret-key-file secrets/litradar.key \
  --project-root .

admin secrets verify \
  --secret-key-file secrets/litradar.key \
  --project-root .
```

迁移在单个 `BEGIN IMMEDIATE` 事务中完成；发现损坏信封时整体回滚。不要在测试或自动启动中对真实数据执行该命令。

## 密钥轮换

轮换要求停写、已验证备份和同时可用的新旧密钥：

```bash
admin secrets rotate \
  --old-key-file secrets/old.key \
  --new-key-file secrets/new.key \
  --project-root .

admin secrets verify \
  --secret-key-file secrets/new.key \
  --project-root .
```

先验证新密钥并更新所有服务挂载，再销毁旧密钥。回滚数据库备份时必须同时恢复与该备份匹配的旧密钥，但两者仍要分开保存。

## 密钥丢失

密钥永久丢失后，密文不可恢复，系统不会降级为明文。恢复路径只有：

- 数据库备份和与其匹配的独立密钥备份
- 清除受保护值并重新录入全部凭据

静态加密不能防御已经取得运行进程、密钥文件或管理员写权限的攻击者。

## 管理员恢复边界

用户表非空时 bootstrap 必须拒绝。管理员忘记密码应由另一个已认证管理员重置，或恢复经过验证的数据库备份；不要直接修改 `is_admin`、删除用户或降低 schema 版本。
