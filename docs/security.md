# 安全说明

本文档记录 Paper Scanner 当前已经实现的认证初始化、密码、限流、网络暴露和容器权限边界。后续密钥加密完成后继续在此更新。

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
