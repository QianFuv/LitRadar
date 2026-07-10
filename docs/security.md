# 安全说明

本文档记录 Paper Scanner 当前已经实现的认证初始化、凭据加密、密码、限流、网络暴露和容器权限边界。

## 集成凭据加密

以下 `data/auth.sqlite` 字段使用 XChaCha20-Poly1305 版本化信封保存：

- `notification_settings.pushplus_token`
- `notification_settings.ai_api_key`
- `notification_settings.ai_backup_api_key`
- `runtime_settings` 中的 `openalex_api_key_pool` 与 `semantic_scholar_api_key_pool`
- `cnki_sessions.session_json`

信封以 `psenc:v1:` 开头，每次写入生成随机 24 字节 nonce，并把表、用户/配置键和字段名作为关联数据。密文被复制到错误字段或错误用户时无法通过认证。空值可以保持为空；所有非空新凭据只允许写入版本化密文。

API、worker、`index`、`notify`、`push` 和调度子进程都必须显式接收 `--secret-key-file PATH`。密钥文件必须是原始二进制的 32 字节，不能是 64 字符十六进制文本、Base64 文本或带换行的口令。进程在绑定端口或进入循环前验证全部已有密文；文件缺失、长度错误、密钥错误、密文被修改或仍有旧明文都会失败关闭，错误消息不包含凭据内容。

### 生成与保存密钥

在仓库外或已被 Git 忽略的 `secrets/` 目录生成：

```bash
mkdir -p secrets
openssl rand -out secrets/paper-scanner.key 32
chmod 600 secrets/paper-scanner.key
wc -c secrets/paper-scanner.key
```

最后一条命令必须输出 `32`。生产环境优先由容器编排 secret、KMS/HSM 解封流程或主机级 secret manager 提供文件。不要把密钥放入 Git、镜像层、Compose YAML、环境变量、SQLite、应用日志、普通备份或与数据库相同的存储位置。根 `.gitignore` 已忽略 `/secrets/`，但这不是密钥管理机制。

### 旧数据库首次迁移

迁移不会在普通启动中自动发生。维护窗口内按以下顺序操作：

1. 停止 API、worker、scheduler 以及可能写入 `auth.sqlite` 的 CLI。
2. 对停止状态的 `data/auth.sqlite` 做独立备份，并确认密钥不在备份目录中。
3. 生成并安全保存 32 字节密钥。
4. 运行 `admin secrets migrate --secret-key-file PATH --project-root PATH`。该命令在单个 `BEGIN IMMEDIATE` 事务中加密旧明文；发现损坏的现有信封时整体回滚。
5. 运行 `admin secrets verify --secret-key-file PATH --project-root PATH`。
6. 把同一密钥文件挂载给 API 和 worker，再启动服务并检查健康状态。

Docker 示例：

```bash
docker compose run --rm api admin secrets migrate --secret-key-file /run/secrets/paper_scanner_key --project-root /app
docker compose run --rm api admin secrets verify --secret-key-file /run/secrets/paper_scanner_key --project-root /app
```

不要在本计划或日常测试中对真实数据自动执行迁移；命令只应由掌握备份和维护窗口的操作员显式运行。

### 轮换与丢失

轮换需要停写、独立备份和两个同时可用的密钥文件：

```bash
admin secrets rotate --old-key-file OLD --new-key-file NEW --project-root PATH
admin secrets verify --secret-key-file NEW --project-root PATH
```

轮换在一个事务内先用旧密钥认证并解密每个值，再用新密钥和新随机 nonce 加密。验证新密钥、更新所有服务挂载并完成一次启动检查前，不要销毁旧密钥。回滚数据库备份时必须同时恢复与该备份匹配的旧密钥，但两者仍应分开保存。

密钥永久丢失时，密文不可恢复；系统会拒绝读取而不是降级为明文。恢复路径只有“数据库备份 + 与其匹配的密钥备份”，或清除并重新录入所有受保护凭据。加密只保护静态数据库、备份或磁盘被单独读取的场景；已取得运行进程权限、密钥文件权限或管理员写权限的攻击者仍可能使用解密后的凭据。

### 脱敏 API 与更新语义

通知设置响应不返回凭据或密文，只返回 `has_*` 布尔值与固定 `••••` 掩码。管理员运行时配置对秘密项返回空 `value`、`has_value` 和固定 `masked_value`。写入语义为：字段缺省保留，JSON `null` 明确清除，非空字符串替换；密码框中的空白字符串也保留已有值。前端必须通过单独的“清除”操作发送 `null`，不能回传响应掩码。

## 管理员初始化

公开 API 永远不能创建第一个管理员。空安装的 `/api/auth/invite-required` 返回：

```json
{
  "required": true,
  "bootstrap_required": true
}
```

管理员只能在能访问 `data/auth.sqlite` 的本机或容器维护环境中创建：

```bash
printf '%s\n' "$ADMIN_PASSWORD" | admin bootstrap --username admin --password-stdin
```

安全边界：

- 命令只接受 `--password-stdin`，不接受密码值参数
- stdout 只返回创建状态和非敏感用户资料，stderr 不输出密码
- storage 使用 `BEGIN IMMEDIATE` 检查并写入；只有用户表为空时成功
- 两个并发 bootstrap 最多一个成功，另一个在看到已存在用户后失败
- bootstrap 不会把已有用户提升为管理员，也不能用作密码恢复命令

Docker 使用 `docker compose run --rm -T api admin ...` 时，应从当前 shell 的安全输入、密码管理器或 secret manager 管道提供密码。不要把实际值写入 Compose、脚本、终端历史、CI 日志或进程参数。

## 注册与密码策略

所有公开注册都需要未使用的邀请码，且只创建普通用户。管理员可在登录后生成邀请码和显式调整用户权限。

注册、用户改密、管理员重置和本机 bootstrap 的新密码至少需要 12 个 Unicode 字符。登录不重新应用长度策略，因此升级前已经存在的短密码哈希仍然有效；用户下次改密时必须使用新策略。

密码使用现有 PBKDF2-HMAC-SHA256 参数保存，数据库只存 hash 和 salt。原始密码只能存在于当前请求或 CLI stdin 的内存中，不应写入日志。

## 登录与注册限流

API 每个进程维护以下固定窗口：

| 桶 | 限制 | 窗口 |
| --- | ---: | ---: |
| 规范化用户名（登录与注册共享） | 5 次 | 5 分钟 |
| 全局登录 | 100 次 | 1 分钟 |
| 全局注册 | 25 次 | 1 分钟 |

用户名会 trim 并转换为 ASCII 小写。成功登录或注册会清除对应用户名桶；失败尝试保留到窗口结束。最多跟踪 4096 个用户名，过期项会清理，容量满时淘汰最旧项。

超过任一桶时，API 返回 `429 Too Many Requests`、数值型 `Retry-After` 和统一正文：

```json
{
  "detail": "Too many authentication attempts; try again later"
}
```

限流不会在 429 中表明用户名是否存在。它是单进程内存防护：API 重启会清空计数，多副本之间不共享。当前 Compose 默认运行一个 API 副本；多副本或公网部署必须在反向代理/API gateway 再配置共享限流。

## 网络暴露

根 Compose 默认使用：

- `127.0.0.1:3000:3000`
- `127.0.0.1:8000:8000`

容器内 API 和前端仍监听 `0.0.0.0`，以便 Compose 内部通信，但不会默认发布到宿主机所有网卡。远程访问应通过明确配置的 TLS 反向代理，并启用安全 Cookie、精确 CORS Origin、MCP Host/Origin 白名单和代理层限流。

生产 API 可附加 `--require-secure-cookies`。该参数不会从环境变量覆盖配置，而是在加载 `data/auth.sqlite` 后验证 `secure_cookies = true`；不满足时进程在绑定端口前失败。Secure Cookie 只能经 HTTPS 正常使用，因此生产部署必须先配置 TLS 反向代理和数据库运行设置，再启用该启动门。

## 容器权限边界

根 Compose 的三个运行容器具备以下共同约束：

- 后端使用固定 `paper` 用户 UID/GID `10001:10001`，前端使用 Node 镜像内置的非 root `node` 用户
- 根文件系统只读，`/tmp` 使用带 `noexec,nosuid` 的 tmpfs；前端另有临时图片缓存 `/app/.next/cache`，后端只有 `/app/data` 是显式可写绑定挂载
- 丢弃全部 Linux capabilities，并启用 `no-new-privileges:true`
- 使用 `restart: unless-stopped`；非零进程退出会重启，手工停止保持停止
- API、前端和持久 worker 心跳分别提供健康检查；健康状态本身不会替代进程退出策略

Linux 原生 Docker Engine 必须在启动前把宿主机 `data` 目录所有权安排给 `10001:10001`，或提供等效 ACL。不要用 root 容器、放宽整个宿主机目录权限或挂载 Docker socket 解决写权限问题。macOS/Windows Docker Desktop 的 bind mount 权限由虚拟化层转换，通常不需要 `chown`。

## 恢复边界

如果用户表非空，bootstrap 必须拒绝运行。丢失管理员密码时，应使用另一个已认证管理员的重置接口或恢复经过验证的数据库备份；不要删除用户、手工降低 schema 版本或修改 `is_admin` 来绕过流程。当前任务不提供离线强制重置，正式备份/恢复工具由后续运维任务实现。
