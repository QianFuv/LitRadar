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
- 同时提供给所有需要解密同一 `auth.sqlite` 的应用实例或按需子命令

`litradar serve`、`litradar index`、`litradar notify`、`litradar push` 和 `litradar scheduler` 都要求 `--secret-key-file`。`litradar admin secrets` 使用相应 key 参数；`litradar admin bootstrap` 和 `litradar admin backup` 不需要密钥。

## 日志数据边界

所有日志级别都禁止密码、部署密钥、Cookie、Bearer token、邀请码、访问令牌、第三方 API key、PushPlus token、认证头、用户名/邮箱、请求或响应 body、URL query、文章/公告/AI 内容、会话 JSON、密文、hash 和完整文件路径。`DEBUG` 或 `TRACE` 不放宽该边界。

允许记录匹配路由、method/status、耗时、计数、工作流阶段、有限 outcome/error kind、provider 名、安全内部数值 ID、服务器 UUID 和调度 run ID。HTTP 请求 ID 由服务器覆盖生成；日志不信任客户端提供的 ID。浏览器记录器只把 allowlist 对象写入本地控制台，不上传 message、stack、promise reason、query、body、token 或 Web Storage。

新增事件或字段必须用唯一 sentinel 覆盖成功与失败路径，验证服务端 JSON、调度子进程 stderr 和浏览器对象都不含秘密或内容。若事故日志发现禁止字段，应立即停止扩大采集、限制证据访问、轮换受影响凭据并按[日志运维](logging.md)的流程处理；不能只依赖后续轮转删除泄漏。

## 持久安全审计

安全审计的权威记录是认证库 v9 的 append-only `security_audit_events`，不是可能丢弃的普通 tracing 队列。认证成功/失败/限流、密码和令牌操作、管理员权限与用户操作、邀请码、调度任务、运行设置和公告变更均使用固定 action/outcome/reason 分类；记录内部 actor/target ID、服务器 request ID 和必要的限流元数据，不记录密码、用户名、token、邀请码、原始 IP、请求体或业务内容。

业务安全变更与必需审计行在同一个 immediate transaction 中提交。审计插入失败时变更整体回滚，API 返回 `503`；认证拒绝和限流在返回前同步追加。失败路径只向 `stderr` 输出固定 `audit.persistence_failed` 分类和进程内计数，不输出 SQLite 原始错误。普通日志过载不会影响已提交审计行。

## 子进程树隔离

调度任务和索引 worker 保留 `Command::new` 的类型化参数边界，不经过 shell。每个直接子进程在启动时同时成为独立 Unix process group 的 leader，或先以 suspended 状态创建、分配到启用 `KILL_ON_JOB_CLOSE` 的 Windows Job Object 后再恢复。这样后续 worker 再派生的抓取进程仍属于同一监管边界。

Unix 取消和超时先向整个 group 发送 SIGTERM，等待 250 ms grace period，再对仍存活的 group 发送 SIGKILL；Windows 原子终止整个 Job Object。所有路径都等待直接子进程完成，Drop/shutdown 也执行强制兜底。spawn/assignment、TERM、force-kill 和 wait 失败只进入固定分类，不把可执行路径或操作系统自由文本写入调度状态与普通日志。Linux 和 Windows CI 都运行真实“子进程派生孙进程”夹具，并以两级监听端口或心跳停止作为回收证据。

手动投递使用相同监管器和私有类型化 `delivery-run` child。SQLite 保存每用户唯一 active run、owner/revision lease、10 分钟绝对 deadline 与取消标志；实例池默认并发 2。child 每个业务边界轮询取消，所有 HTTP timeout 受剩余 deadline 限制。dispatcher 在取消 grace 后回收完整树；deadline 到达时直接强制回收。若强制回收时不能证明外部副作用未发生，任务固定为 `unknown` 且不允许自动重试。

默认保留 180 天，可通过 `audit_retention_days` 设置 1–3650 天。启动后立即检查并每 24 小时检查一次；跨实例持久窗口和每事务 10,000 行上限避免无界删除。系统不暴露远程审计 API，查询、导出和取证只能使用受控的只读数据库副本；具体 SQL 与告警规则见[日志运维](logging.md)。`auth.sqlite` 固定备份范围包含审计历史和 maintenance 标记。

## 数据库凭据加密

以下非空字段使用 `litradarenc:v1:` XChaCha20-Poly1305 认证信封：

- `notification_settings.pushplus_token`
- `notification_settings.ai_api_key`
- `notification_settings.ai_backup_api_key`
- `runtime_settings.openalex_api_key_pool`
- `runtime_settings.semantic_scholar_api_key_pool`
- `cnki_sessions.session_json`

每次写入生成随机 24 字节 nonce，并把表、行/配置键和字段名作为关联数据。密文复制到其他用户或字段后无法通过认证。

统一服务在绑定端口和启动调度前验证现有秘密值，按需子命令在业务工作前验证。密钥缺失、长度错误、密文损坏、密钥不匹配或残留明文都会使启动失败；错误消息不包含凭据。

当前二进制只接受 `litradarenc:v1:` 信封。改名前的信封格式不会被读取或自动迁移；`litradar admin secrets migrate` 只把明文转换为当前格式。

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
  litradar admin bootstrap \
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

- 公开注册始终需要处于 active 状态的邀请码，只创建普通用户；过期、撤销或达到使用上限均在注册事务内拒绝。
- 普通用户邀请码默认有效 7 天、最多使用 1 次；同一用户最多一个未撤销发行，rotate 会在一个 immediate transaction 中撤销旧码并创建替代码。
- 管理员可把有效期覆盖为当前时间后最多 365 天，把最大使用次数覆盖为 `1..=1000`；撤销只写 `revoked_at`，不物理删除兑换历史。
- 注册在同一 immediate transaction 中创建用户、递增 `use_count`、追加 `invite_code_uses` 和安全审计；并发争用最后一个名额时只有一个事务能提交。
- 用户名长度 `3..32`，只允许字母、数字和下划线。
- bootstrap、注册、改密和管理员重置的新密码至少 12 个 Unicode 字符。
- 既有短密码哈希仍可登录，直到下次改密。
- 新密码使用 PHC 格式 Argon2id（`m=19456 KiB,t=2,p=1`）；API 的密码 KDF 使用独立并发 2 gate，避免 160 MiB 容器内出现不受限的并行内存消耗。
- 旧 PBKDF2-HMAC-SHA256 hex+salt 行继续验证；正确登录后以原 hash+salt 为 CAS 条件升级为 Argon2id，错误密码不会升级，并且升级不撤销现有 token。
- 用户名不存在时仍对固定有效 dummy PHC 执行同参数 Argon2id 验证，再返回统一认证失败。
- 密码变更和管理员重置在一个 `BEGIN IMMEDIATE` 事务内更新 hash/salt 并撤销该用户全部令牌；任一步失败都会整体回滚。
- salt、浏览器会话、Personal Access Token、邀请码和手动作业 ID 均由操作系统 CSPRNG 生成，不依赖 SQLite `randomblob()`。
- 浏览器登录令牌只通过 `HttpOnly`、`SameSite=Lax` 的 `litradar_session` Cookie 传输。
- 浏览器会话固定 7 天到期、登录时轮换且不滚动续期；Personal Access Token 的有效期由创建时显式选择。
- 用户创建的长期令牌只通过 Bearer 请求头用于外部客户端。
- 令牌不得放入 URL 查询参数。
- `expires_at <= now` 统一视为已过期；验证路径会拒绝并清理边界时刻的令牌。
- 含密码、salt、原始 token、邀请码或邀请码关联用户名的认证与管理类型使用脱敏 Debug，避免后续诊断误打印秘密。

注销对 SQLite busy/locked 使用 250 ms busy timeout、25 ms 间隔和最多一次重试。携带浏览器 Cookie 的 `/api/auth/logout` 与 `/api/auth/logout-all` 在所有响应中都清除 Cookie；`logout` 的 `401` 只表示该令牌在请求前已经无效，浏览器可把它视为幂等成功。若数据库删除或必需审计仍无法提交，则返回 `503 session_revocation_unconfirmed` 和 request ID。前端只清理非秘密本地快照，并把未确认标记保存在固定 shape 的 localStorage 元数据中；刷新不能把它改写为成功。恢复操作要求重新认证，再调用 `/api/auth/logout-all` 原子撤销该用户的全部登录令牌和 Personal Access Token。旧 Cookie 已被清除，不存在安全的“重试原注销请求”路径。

## 登录和注册限流

每个 `litradar serve` 进程按客户端 IP → 当前操作的规范化用户名 → 高阈值全局熔断器顺序检查 token bucket。前置桶拒绝后不会消耗后续额度：

| 桶                     | burst | 补充速率 | 内存 key 上限   |
| ---------------------- | ----: | -------- | --------------- |
| 登录客户端 IP          |    30 | 1/s      | IP 合计 8192    |
| 注册客户端 IP          |     5 | 1/60s    | IP 合计 8192    |
| 每种操作的规范化用户名 |     5 | 1/60s    | 用户名合计 4096 |
| 全局登录熔断器         |  1000 | 100/s    | 单例            |
| 全局注册熔断器         |   250 | 25/s     | 单例            |

用户名会 trim、截断为注册策略允许的最多 32 个 code point，再转为 ASCII 小写。IP/用户名 map 使用真正的最近使用顺序淘汰，旋转输入不能使 key 数无限增长。成功登录或注册只清除该操作对应的用户名桶，不清除 IP 或全局额度。

`trusted_proxy_cidrs` 默认空。任意 `Forwarded`/`X-Forwarded-For` 都不能改变不可信直连 peer 的分桶；只有直连地址命中明确 CIDR 时才按右到左可信链取客户端地址。标准 `Forwarded` 优先，链必须全部是数值 IP/可选端口；无效链回退到直连代理地址的共享桶。不要把任意公网范围加入可信列表，可信代理也必须覆盖而不是盲目追加来自客户端的转发头。

超过限制返回统一 `429`、数值 `Retry-After` 和相同 detail，不泄露用户名是否存在。结构化事件包含固定 `reason`、`bucket`、`source_class`、递增 `rejected_count` 和服务器生成的 `request_id`，不记录原始 IP、用户名或转发头。策略可由严格的 `auth_rate_limit_policy` JSON 调整，但 parser 要求全局桶容量/补充速率始终高于前置桶。

这些桶和计数只在单进程内存中，重启会清空。多副本或公网部署必须在可信网关使用共享限流（例如 Redis-backed gateway policy）；应用内全局桶只是额外熔断器，不能代替跨实例控制。

## Cookie、CORS 和 MCP

默认 `secure_cookies=false`，适合 loopback HTTP。生产 HTTPS 应先把数据库设置改为 `true`，再用 `litradar serve --require-secure-cookies` 作为启动门；不满足时应用在绑定端口前失败。

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

管理员提交无效 CORS/MCP Origin 时，API 返回 `400` 且整份更新不落库；有效修改在下次 `litradar serve` 启动时生效。旧版本或库外修改留下的无效行会在监听端口绑定前使启动明确失败，不会静默忽略或自动修复，从而避免意外放宽网络策略。

全局配置详见[运行配置参考](../reference/configuration.md)。

### 响应安全策略

生产前端构建在 `next build` 后生成 `web/csp-hashes.json`，记录每个导出 HTML 的完整 SHA-256 和所有内联脚本的 CSP SHA-256。服务启动时会递归重新读取全部 HTML，并要求文件集合、文件摘要、脚本顺序和全局哈希集合与清单完全一致；清单缺失、损坏、过大、过期，静态目录缺少 HTML 或包含符号链接都会在绑定端口前失败。部署时必须把同一次构建产生的 `web/` 作为整体复制，不能单独替换 HTML。

所有静态与后端响应统一包含：

- `Content-Security-Policy`：脚本只允许同源外部文件和已复算的精确 SHA-256，不包含脚本 `unsafe-inline`；样式暂时保留 `style-src 'self' 'unsafe-inline'` 以兼容当前 Next.js 静态导出。
- `X-Content-Type-Options: nosniff`
- `Referrer-Policy: same-origin`
- `Permissions-Policy: camera=(), microphone=(), geolocation=(), payment=(), usb=()`
- `X-Frame-Options: DENY`，并同时使用 CSP `frame-ancestors 'none'`。

`Strict-Transport-Security: max-age=31536000` 只在使用 `--require-secure-cookies` 的 hardened 模式下由应用发送。该模式同时要求数据库中的 `secure_cookies=true`；仅在确认外部入口始终使用 HTTPS 后启用。TLS 反向代理不得删除或用更宽松策略覆盖这些 Header。

所有 `/api/auth` 和 `/api/auth/*` 响应无论状态或是否携带凭据，都强制使用 `Cache-Control: no-store` 与 `Pragma: no-cache`。这包括成功、输入错误、认证失败、限流和内部失败，避免浏览器或中间缓存保存认证状态。

### 静态 Web 缓存

生产导出的 Web 文件是公开构建产物，不得包含部署密钥或用户秘密。Rust 按以下边界设置缓存：

- 成功的 `/_next/static/*` 哈希资源使用 `public, max-age=31536000, immutable`，即使请求携带会话 Cookie 也不会变成用户专属内容。
- 页面、导航 payload 和导出的 404 使用 `no-cache`，以便浏览器重新验证版本。
- 受保护 API、携带 Bearer/Cookie 的非静态响应和 `401` 继续使用 `private, no-store`。
- 所有认证路径使用更严格且状态无关的 `no-store` 与 `Pragma: no-cache`。
- 支持 gzip 的客户端读取镜像内预压缩兄弟文件；不支持的客户端读取原文件。

`/api`、`/mcp`、`/docs` 和 `/openapi.json` 始终由后端路由优先处理，未知路径不会借静态 fallback 读取项目数据或密钥。

## 服务器出站请求

普通用户不能配置任意 AI URL。管理员通过 `ai_allowed_base_urls` 维护准确 HTTPS base URL 目录，默认空目录会禁用 AI 投递；用户只从 `GET /api/tracking/ai-endpoints` 返回值中选择。API 与 storage 写入事务做准确成员校验，worker 在每次实际 AI 请求前再次读取目录，避免运行中撤销的配置用于后续重试、格式回退或摘要请求。

AI 与 PushPlus 共用以下出站边界：

- DNS 查询在请求总截止时间内通过固定并发解析器执行，并把已校验地址固定到本次 client
- 任一解析结果为 loopback、RFC1918、link-local、unspecified、multicast、IPv6 ULA、NAT64/6to4 或其他特殊用途地址时整次请求失败
- 禁用环境代理和自动重定向，不会跟随公网 URL 跳转到内网
- 只接受未压缩 JSON 成功响应，响应体硬上限为 2 MiB
- 非 2xx 响应不读取 body；错误只保留固定分类、HTTP 状态和可选上游 request ID
- 请求对象的 Debug 输出不包含 API key、prompt、文章、通知正文或 URL query

AI/PushPlus 只重试连接失败、timeout 和 `429/502/503/504`；数值 `Retry-After` 上限 60 秒，其他情况使用指数 full jitter。`400/401/403` 和 PushPlus `500` 都只尝试一次。手动任务跨主备 Endpoint、格式和摘要请求共享 8 次 AI HTTP 预算；输出格式降级只发生在成功响应的明确兼容性失败之后。

应用层策略不能替代基础设施隔离。公网部署仍应在容器、主机或云网络层设置 egress ACL，只放行确有需要的 AI/PushPlus 目标和 DNS/TLS 基础设施。

## 供应链门禁

所有 pull request、非 `main` 分支推送和每周计划任务运行 `.github/workflows/security.yaml` 与 `.github/workflows/codeql.yaml`。`main` 的镜像发布工作流复用这两个工作流；RustSec、cargo-deny、OSV、完整 Git 历史 Gitleaks、workflow pin 检查或任一语言 CodeQL 非零告警失败时，不会进入镜像构建与推送。

执行边界如下：

- `cargo-audit 0.22.2` 使用 `--deny warnings` 检查 `Cargo.lock`；`cargo-deny 0.20.2` 同时执行 advisory、license、ban 和 source policy。
- `OSV-Scanner 2.3.8` 只读取已提交的 `Cargo.lock` 与 `app/pnpm-lock.yaml`；发行二进制先按上游 SHA-256 清单验证。
- `Gitleaks 8.30.1` 在 `fetch-depth: 0` checkout 上扫描所有可达提交，SARIF 同时进入 artifact 和 GitHub code scanning。
- CodeQL 使用 `security-extended` 分别分析 Rust 与 JavaScript/TypeScript；SARIF 上传后，本工作流统计结果并要求零告警。
- `actionlint 1.7.12` 校验所有 workflow。第三方 `uses:` 必须是 40 位小写提交 SHA，并保留已审核版本注释；同仓库 `./.github/workflows/...` 是唯一不需要 SHA 的引用。
- GitHub Action、Cargo、pnpm 和 Docker 更新由 `.github/dependabot.yml` 每周提出；更新 pull request 必须重新通过上述门禁，不能把执行引用改回 tag 或 branch。

`deny.toml` 默认拒绝未允许的许可证、未知 registry/git source、wildcard 外部依赖和任何 advisory。`osv-scanner.toml` 只允许逐 advisory 例外。每个例外必须包含 owner、具体不可利用理由和到期日；版本或作用域扩大时必须新审，过期后扫描自动恢复阻断。当前没有 RustSec/source 忽略，唯一 cargo-deny 许可证例外精确限定到 `webpki-roots@1.0.8` 的 TLS 根数据许可证。`.gitleaksignore` 只能保存逐 commit/path/rule/line fingerprint，并同样记录 owner、非凭据证据和复审日期；禁止按整条规则或路径排除。

仓库管理员还必须在 GitHub ruleset 中把四个 supply-chain job 和两个 CodeQL language job 设为 required checks，并启用 code-scanning merge protection、secret scanning 与 push protection。workflow 文件不能替代这些仓库级设置；Gitleaks 是独立的纵深防御，而不是关闭 GitHub secret scanning 的理由。

容器发布工作流同样属于阻断门禁：

- Dockerfile frontend 与 Node/Rust/Debian 基础镜像都固定到 reviewed digest；tag 只保留可读性和 Dependabot 更新入口。
- Buildx 把一次构建以无 tag digest 推入 GHCR；hardened smoke 必须重新拉取该 `repository@sha256:...`，并验证实际 RepoDigest、固定 UID/GID、只读根、完整 capability drop、no-new-privileges、loopback 端口、Docker health、只读密钥和唯一持久可写数据卷。
- smoke 成功后才为同一 digest 生成 SPDX SBOM 与 SLSA provenance、写入 GitHub artifact attestations，并用 workflow OIDC 进行 Cosign keyless signing；workflow 随即用精确 certificate identity、issuer 和 digest 重新验证三类证明。
- 只有上述步骤全部成功，才用 `imagetools create --prefer-index=false` 为同一 digest 创建不可变的 `sha-<full commit>` tag，并更新可变的 `latest` tag。既有 full-commit tag 指向其他 digest 时发布失败；两个发布 tag 都必须解析到经过验证的 digest，生产配置仍不接受 `latest`。
- `compose.production.yaml` 删除本地 build/ports，要求 64 位 digest 并强制 `--require-secure-cookies`。生产运行仍必须先独立验证 Cosign、provenance 和 SBOM attestation。

## 网络暴露

根 Compose 仅发布：

- `127.0.0.1:8000:8000`

该 Rust 入口同时提供 Web、REST、Swagger/OpenAPI 和 MCP。容器内监听 `0.0.0.0` 只用于 Compose 网络通信。远程访问应经 TLS 反向代理，并同时配置 Secure Cookie、准确 CORS/MCP 白名单和共享限流。不要直接把宿主机端口改为所有网卡。

## 容器边界

唯一的 `litradar` 常驻容器使用无后缀镜像：

- 使用 UID/GID `10001:10001`；最终镜像只有 `/usr/local/bin/litradar`，没有 Node.js 运行时
- 根文件系统只读
- `/tmp` 使用 `noexec,nosuid,nodev` tmpfs
- 只允许 `/app/data` 持久写入；`/app/web` 保持只读
- 丢弃全部 Linux capabilities
- 启用 `no-new-privileges:true`
- 镜像定义 `/health/ready` Docker health check；发布 smoke 另行探测根 Web、OpenAPI 和 auth cache/Header 边界
- 生产覆盖文件不发布宿主机端口，并要求数据库 `secure_cookies=true`

不要通过 root 容器、开放整个宿主机目录或挂载 Docker socket 解决权限问题。

## 旧明文迁移

普通启动不会自动迁移明文凭据。维护窗口内：

1. 停止 `litradar serve` 和所有可能写 `auth.sqlite` 的按需子命令。
2. 创建并验证独立数据库备份。
3. 生成并单独保存部署密钥。
4. 执行迁移。
5. 执行验证。
6. 给统一应用和后续按需命令提供同一密钥后再启动。

```bash
litradar admin secrets migrate \
  --secret-key-file secrets/litradar.key \
  --project-root .

litradar admin secrets verify \
  --secret-key-file secrets/litradar.key \
  --project-root .
```

迁移在单个 `BEGIN IMMEDIATE` 事务中完成；发现损坏信封时整体回滚。不要在测试或自动启动中对真实数据执行该命令。

## 密钥轮换

轮换要求停写、已验证备份和同时可用的新旧密钥：

```bash
litradar admin secrets rotate \
  --old-key-file secrets/old.key \
  --new-key-file secrets/new.key \
  --project-root .

litradar admin secrets verify \
  --secret-key-file secrets/new.key \
  --project-root .
```

先验证新密钥并更新应用密钥挂载，再销毁旧密钥。回滚数据库备份时必须同时恢复与该备份匹配的旧密钥，但两者仍要分开保存。

## 密钥丢失

密钥永久丢失后，密文不可恢复，系统不会降级为明文。恢复路径只有：

- 数据库备份和与其匹配的独立密钥备份
- 清除受保护值并重新录入全部凭据

静态加密不能防御已经取得运行进程、密钥文件或管理员写权限的攻击者。

## 管理员恢复边界

用户表非空时 bootstrap 必须拒绝。管理员忘记密码应由另一个已认证管理员重置，或恢复经过验证的数据库备份；不要直接修改 `is_admin`、删除用户或降低 schema 版本。

管理员授权或删除用户时，路由层的“不能撤销/删除自己”只提供提前反馈。storage 在一个 `BEGIN IMMEDIATE` 中重新读取 actor 是否仍为管理员、target 是否存在以及当前管理员计数，再执行写入。撤权或删除的提交结果必须至少保留一个管理员；并发交叉撤权/删除中只有一个事务可以成功，另一个会因 actor 已失权或已删除而返回固定 `403` 分类。target 不存在映射为 `404`，最后管理员不变量冲突映射为 `409`，三者不会降级为通用 `500`。
