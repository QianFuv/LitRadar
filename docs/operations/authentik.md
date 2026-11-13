# authentik 单点登录

LitRadar 支持用户名或邮箱加密码登录，也支持 authentik OpenID Connect 登录。
首次部署仍先通过 `litradar admin bootstrap` 创建本地管理员，再启用单点登录。
authentik 不授予 LitRadar 管理员权限；管理员身份始终在 LitRadar 本地管理。
以下示例假定公开地址为 `https://papers.example.org`。

## authentik 提供程序

在 authentik 创建 **OAuth2/OpenID Provider** 和关联的 Application：

- Client type：**Confidential**，保存 Client ID 和 Client Secret。
- 授权流程：authorization code；LitRadar 使用 PKCE S256 和 `client_secret_basic`。
- Redirect URI：使用严格匹配，填写 `https://papers.example.org/api/auth/oidc/callback`。原生客户端也由服务端回调，无需额外 redirect URI。
- Scope mappings：`openid profile email`，userinfo 必须返回 `sub`、`email` 和布尔值 `email_verified`。
- Subject mode：`sub_mode=user_uuid`，以稳定 UUID 标识用户；不要使用可更改的用户名或邮箱作为 subject。
- 只有经 authentik 验证（例如邮箱 OTP）的当前地址才能声明 `email_verified=true`；注册限制（邀请、邮箱域名等）由 authentik 的注册流程负责。
- 建议禁止用户在 authentik 中自行更改用户名和邮箱，避免身份漂移。

将提供程序的 **OpenID Configuration Issuer** 原样填入 LitRadar 的 issuer 设置，
包括其路径和末尾斜杠。服务从该 issuer 的
`/.well-known/openid-configuration` 获取端点；发现文档缓存最多五分钟。
不要填写 authentik 实例首页来替代实际 issuer。

## LitRadar 运行设置

在管理员运行设置的 `single_sign_on` 分组配置：

| 设置                 | 值                                                  |
| -------------------- | --------------------------------------------------- |
| `oidc_issuer_url`    | authentik 提供程序的 issuer URL                     |
| `oidc_client_id`     | confidential client 的 Client ID                    |
| `oidc_client_secret` | Client Secret；使用部署密钥加密保存，界面仅显示掩码 |
| `oidc_display_name`  | 默认 `authentik`，可更改登录按钮名称                |
| `public_base_url`    | `https://papers.example.org`                        |

issuer、client id、client secret 和 public base URL 全部非空才启用 OIDC。
设置在下一次请求生效；更改流程相关设置后，应重新发起登录。
生产环境 issuer、公开地址和发现端点都要求 HTTPS；只有显式 `--development`
启动才允许 HTTP。`secure_cookies` 控制会话和 OIDC 状态 Cookie 的 Secure 属性，
生产环境应设为 `true`。反向代理、可信代理与 cookie 设置见[安全说明](security.md)。

浏览器流程使用十分钟的 HMAC 签名 `litradar_oidc` Cookie，签名密钥来自已有部署密钥。
Cookie 为 HttpOnly、SameSite=Lax，路径限于 `/api/auth/oidc`，回调时清除。
回调复用普通 LitRadar 会话签发逻辑；个人访问令牌和 MCP Bearer 认证保持原有规则。
原生客户端流程见[原生客户端 OIDC](security.md#原生客户端-oidc)。

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

ID token 由服务端从 token endpoint 取得，并由 go-oidc 按发现文档的 `jwks_uri` 验证签名；
同时检查 `iss`、`aud`、`azp`、`exp`、`iat` 和 `nonce`，并要求 userinfo `sub` 一致。
完整验证规则见[安全说明](security.md#oidc-身份验证)。

回调只暴露有界错误码：`unavailable`、`state`、`denied`、`provider`、
`email_unverified`、`email_conflict`、`account_conflict`、`expired_session`。
登录失败返回 `/login?oidc_error=<code>`，绑定失败返回 `/?oidc_link=<code>`。
安全审计记录 `oidc_login`、`oidc_provision`、`oidc_link`、`oidc_unlink`、`oidc_native_exchange`、`email_change`，
不保存授权码、token、nonce、state、邮箱或 client secret。

## 回调访问日志

OIDC 回调 query 含一次性授权码和 state。LitRadar 自身 HTTP 日志只记录匹配路由，不记录 URL query；
仓库自带的 Caddy `gateway` 未启用访问日志。若在 LitRadar 前另有记录访问日志的反向代理、CDN 或 WAF，
发布前应对 `/api/auth/oidc/callback` 禁止记录完整 query，同时保持原有转发和请求头配置。

以 Nginx（包括 Nginx Proxy Manager 的 Advanced 配置）为例，复制当前生效的代理 location，
改为精确路径并加入：

```nginx
location = /api/auth/oidc/callback {
    # Keep the same proxy_pass and proxy_set_header directives as the main location.
    access_log off;
}
```

保存前校验代理配置，保存后通过一次实际登录确认回调仍能转发且访问日志不再记录它。
