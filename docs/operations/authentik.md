# authentik 单点登录

LitRadar 支持用户名或邮箱加密码登录，也支持 authentik OpenID Connect 登录。
首次部署仍先通过 `litradar admin bootstrap` 创建本地管理员，再启用单点登录。
authentik 不授予 LitRadar 管理员权限；管理员身份始终在 LitRadar 本地管理。

## authentik 提供程序

在 authentik 创建 **OAuth2/OpenID Provider** 和关联的 Application：

- Client type：**Confidential**，保存 Client ID 和 Client Secret。
- 授权流程：authorization code；LitRadar 使用 PKCE S256 和 `client_secret_basic`。
- Redirect URI：使用严格匹配，填写 `https://paper.qianfuv.fun/api/auth/oidc/callback`。
- Scope mappings：`openid profile email`，userinfo 必须返回 `sub`、`email` 和布尔值 `email_verified`。
- Subject mode：`sub_mode=user_uuid`，以稳定 UUID 标识用户；不要使用可更改的用户名或邮箱作为 subject。
- 保留现有注册入口的邀请、`mails.neu.edu.cn` 邮箱和邮箱 OTP 规则；只有当前已验证地址才能声明 `email_verified=true`。
- 不允许用户自行更改用户名和邮箱，维持既定的身份管理策略。

将提供程序的 **OpenID Configuration Issuer** 原样填入 LitRadar 的 issuer 设置，
包括其路径和末尾斜杠。服务从该 issuer 的
`/.well-known/openid-configuration` 获取端点；发现文档缓存最多五分钟。
不要填写 authentik 实例首页来替代实际 issuer。

## LitRadar 运行设置

在管理员运行设置的 `single_sign_on` 分组配置：

| 设置 | 值 |
|---|---|
| `oidc_issuer_url` | authentik 提供程序的 issuer URL |
| `oidc_client_id` | confidential client 的 Client ID |
| `oidc_client_secret` | Client Secret；使用部署密钥加密保存，界面仅显示掩码 |
| `oidc_display_name` | `authentik`，可更改登录按钮名称 |
| `public_base_url` | `https://paper.qianfuv.fun` |

issuer、client id、client secret 和 public base URL 全部非空才启用 OIDC。
设置在下一次请求生效；更改流程相关设置后，应重新发起登录。
生产环境 issuer、公开地址和发现端点都要求 HTTPS；只有显式 `--development`
启动才允许 HTTP。`secure_cookies` 控制会话和 OIDC 状态 Cookie 的 Secure 属性，
生产环境应设为 `true`。反向代理、可信代理与 cookie 设置见[安全说明](security.md)。

浏览器流程使用十分钟的 HMAC 签名 `litradar_oidc` Cookie，签名密钥来自已有部署密钥。
Cookie 为 HttpOnly、SameSite=Lax，路径限于 `/api/auth/oidc`，回调时清除。
回调复用普通 LitRadar 会话签发逻辑；个人访问令牌和 MCP Bearer 认证保持原有规则。

## 账号建立、绑定和恢复

首次外部登录使用 `(issuer, sub)` 查找身份。没有身份时，仅在 userinfo 邮箱已验证且
未被任何账号使用的情况下创建新的非管理员 OIDC 账号。用户名优先使用 authentik 的
`preferred_username`（需符合 3–32 位字母、数字、下划线规则且未被占用），否则生成
`oidc_` 开头的随机用户名。不会按用户名或邮箱自动绑定。邮箱冲突时，先用原账号密码登录，再从账号设置绑定 authentik。

本地账号绑定后可以继续使用密码或 authentik；解除绑定保留密码入口。
绑定必须从已认证的本地浏览器会话开始，并在回调时重新检查用户和凭据代次。
OIDC 创建的账号仅通过 authentik 登录，不能修改本地邮箱、修改密码或通过管理员重置密码。
每次登录使用已验证的 userinfo 邮箱同步 OIDC 账号地址；新地址已被占用时保留原地址。
本地账号邮箱不会随绑定提供程序的邮箱变化而更新。

ID token 仅来自服务端通过验证 TLS 的 token endpoint；验证 `iss`、`aud`、`azp`、
`exp`、`iat` 和 `nonce`，并要求 userinfo `sub` 一致。当前按已批准的安全边界省略
ID token 签名验证；若以后接受浏览器传来的 token，必须先加入 JWKS 签名验证。

回调只暴露有界错误码：`unavailable`、`state`、`denied`、`provider`、
`email_unverified`、`email_conflict`、`account_conflict`、`expired_session`。
登录失败返回 `/login?oidc_error=<code>`，绑定失败返回 `/?oidc_link=<code>`。
安全审计记录 `oidc_login`、`oidc_provision`、`oidc_link`、`oidc_unlink`、`email_change`，
不保存授权码、token、nonce、state、邮箱或 client secret。

## Nginx Proxy Manager 回调日志

OIDC 回调 query 含一次性授权码和 state。发布前，在 Nginx Proxy Manager 的
`paper.qianfuv.fun` Proxy Host 中为 `/api/auth/oidc/callback` 配置单独的精确路径 location，
复用现有 `litradar:8000` 转发和请求头配置，并在这个 location 内设置：

```nginx
access_log off;
```

可在 Proxy Host 的 Advanced 配置中复制当前生效的代理 location，改为
`location = /api/auth/oidc/callback` 并加入上述指令；保持原有转发头和代理行为。
使用该 NPM 版本生成的配置作为模板，避免另建 location 时丢失代理设置。
保存前校验 Nginx 配置，保存后通过一次实际登录确认回调仍能转发且 access log 不再记录它。
若有上游 CDN、WAF 或其他代理，也应对该路径禁记完整 query。
LitRadar 自身 HTTP 日志只记录匹配路由，不记录 URL query。

本文描述操作要求；不创建 authentik 资源、不写生产运行设置、不修改 NPM 配置，也不部署服务。
