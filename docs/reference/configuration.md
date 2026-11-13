# 运行配置参考

LitRadar 不使用单一 `.env` 作为配置中心。不同配置来源服务于不同边界，本页说明其职责、默认值和优先级。

## 配置来源

| 来源                                | 范围           | 典型内容                                                 |
| ----------------------------------- | -------------- | -------------------------------------------------------- |
| CLI 参数                            | 一个子命令调用 | 路径、监听地址、调度间隔、并发、超时、dry-run            |
| `data/auth.sqlite.runtime_settings` | 后端全局       | key 池、Provider 代理/路由/顺序、CORS、MCP、Cookie、日志 |
| `notification_settings`             | 单个用户       | AI、PushPlus、偏好、投递方式                             |
| 固定前端/镜像/进程协议              | 构建与运行时   | 同源 API、开发代理、只读 Meta bundle、父子进程日志关联   |
| 部署密钥文件                        | 一个部署       | 认证和解密数据库秘密值                                   |

服务端不把 LitRadar 自定义环境变量作为通用配置中心：全局可配置业务值通过管理员前端写入数据库，用户级通知/追踪值通过个人设置中心写入数据库。Meta bundle 路径、日志设置和父子进程关联没有环境变量覆盖。现有的环境输入只有：

- `litradar index` 在数据库 token 为空时读取 `LITRADAR_CNKI_CAPTCHA_TOKEN` 作为单次国内 CNKI 探测输入；
- 征稿采集辅助程序路径 `LITRADAR_OBSCURA_PATH`、`LITRADAR_PDFTOTEXT_PATH`，见[征稿 CLI](cli.md#cfp)；
- 本地开发时 Vite 读取 `LITRADAR_BACKEND_URL` 作为开发代理的后端地址，默认 `http://127.0.0.1:8001`；
- Web host 与 Compose 的部署变量，见[前端网络边界](#前端网络边界)。

标准测试工具和操作系统进程发现保留各自的环境输入。

## 部署密钥文件

`--secret-key-file` 指向恰好 32 字节的原始文件。它不是 `runtime_settings`，不存入 SQLite，也不能由环境变量回退。

需要该文件的公共入口：

- `litradar serve`
- `litradar index`
- `litradar notify`
- `litradar push`
- `litradar scheduler`
- `litradar admin secrets migrate/verify`（`rotate` 改用 `--old-key-file` 和 `--new-key-file`）

`litradar admin bootstrap`、`litradar admin backup`、`litradar admin index optimize-storage`、`litradar cfp` 和 `litradar openapi` 不需要密钥。生成、轮换和恢复要求见[安全说明](../operations/security.md)。

## 官方 Meta 打包路径

发布 Docker 镜像固定把官方 CSV 和 `bundle-manifest.json` 复制到 `/usr/share/litradar/meta`；二进制发行包则把它们放在可执行文件旁的 `assets/meta`。持久副本始终位于 `<project-root>/data/meta`。应用依次检查 `/usr/share/litradar/meta/bundle-manifest.json` 和可执行文件所在目录的 `assets/meta/bundle-manifest.json`，使用第一个存在的 bundle；这两个路径都不是 `runtime_settings`、秘密、普通运维覆盖项、环境变量或 CLI 参数。

manifest 存在时，`serve` 和普通 `index` 会在认证库迁移后验证整个 bundle，再按 manifest hash 创建、接管或升级官方文件。自定义的同名文件和 manifest 外文件保持不变；结果产生 `storage.managed_meta.prepared` 事件。bundle 格式/hash 非法、持久目标不是普通目录/文件或检测到版本降级时，命令在后续工作前失败。

本地构建的可执行文件旁通常没有 `assets/meta`，两处都没有 manifest 时发现结果为 no-op，不要求 `/usr/share/litradar/meta` 存在。此时运维人员必须自行创建 `<project-root>/data/meta`；目录缺失时索引明确失败，目录存在但没有选中的 CSV 时返回 `skipped`。

## 全局运行设置

管理员通过 `GET/PUT /api/admin/runtime-settings` 或前端管理页维护以下各项。响应中的 group、control、apply mode、allowed values 和秘密标记是前端控件的权威元数据；管理页必须逐项呈现，不能用硬编码字段子集代替。

这些字段的字段名、默认值、parser/serializer、秘密标记和 UI 元数据由 storage 层同一 registry 定义。管理 API 写入、`serve` 启动、命令启动和 observability 初始化都调用该 registry；不存在“保存成功但下次启动才发现语法无效”的第二套校验。registry 的所有默认值和往返序列化均由穷举测试覆盖。

| 字段                               | 默认值                     | 秘密 | 前端控件                          | 生效时机 | 使用者                            |
| ---------------------------------- | -------------------------- | ---: | --------------------------------- | -------- | --------------------------------- |
| `openalex_api_key_pool`            | 空                         |   是 | 可逐项增删的掩码秘密池            | 下一命令 | scholarly 索引                    |
| `semantic_scholar_api_key_pool`    | 空                         |   是 | 可逐项增删的掩码秘密池            | 下一命令 | scholarly 索引                    |
| `cnki_captcha_token`               | 空                         |   是 | 单值密码文本                      | 下一命令 | 国内 CNKI captcha（jfbym）token   |
| `provider_proxy_url`               | 空                         |   是 | 单值密码文本                      | 重启进程 | 启用代理的内置 Provider HTTP      |
| `crossref_mailto_pool`             | 空                         |   否 | 有序字符串列表                    | 下一命令 | Crossref polite 联系邮箱          |
| `cors_allowed_origins`             | 空                         |   否 | 有序字符串列表                    | 重启进程 | API credentialed CORS             |
| `mcp_allowed_hosts`                | `localhost,127.0.0.1,::1`  |   否 | 有序字符串列表                    | 重启进程 | MCP Host 白名单                   |
| `mcp_allowed_origins`              | 空                         |   否 | 有序字符串列表                    | 重启进程 | 浏览器 MCP Origin 白名单          |
| `secure_cookies`                   | `false`                    |   否 | 布尔开关                          | 重启进程 | `litradar_session` 的 Secure 标志 |
| `trusted_proxy_cidrs`              | 空                         |   否 | IPv4/IPv6 CIDR 列表               | 重启进程 | 可信反向代理直连网络              |
| `auth_rate_limit_policy`           | 见下文                     |   否 | 严格 JSON 文本                    | 重启进程 | 登录/注册分层 token bucket        |
| `audit_retention_days`             | `180`                      |   否 | `1–3650` 的整数                   | 下一请求 | 持久安全审计保留                  |
| `ai_allowed_base_urls`             | 空                         |   否 | 有序 HTTPS URL 列表               | 下一请求 | 用户可选择的 AI Endpoint 目录     |
| `delivery_worker_concurrency`      | `2`                        |   否 | `1–16` 的整数                     | 重启进程 | 手动投递子进程池                  |
| `provider_proxy_policy`            | 内置 Provider 均为 `false` |   否 | 按能力目录生成的独立开关          | 重启进程 | 逻辑 Provider 到代理启用状态      |
| `index_provider_routes`            | 三个官方目录的默认映射     |   否 | 每个 catalog 的能力过滤单选       | 下一命令 | CSV stem 到索引 Provider          |
| `article_abstract_provider_orders` | 见下文                     |   否 | 默认顺序 + catalog 继承/排序/禁用 | 下一请求 | 在线摘要页 fallback               |
| `article_fulltext_provider_orders` | 见下文                     |   否 | 默认顺序 + catalog 继承/排序/禁用 | 下一请求 | 在线全文 fallback                 |
| `log_format`                       | `json`                     |   否 | `json` / `compact` 单选           | 重启进程 | 结构化日志格式                    |
| `log_filter`                       | 见[日志设置](#日志设置)    |   否 | 文本                              | 重启进程 | Go regexp 字段过滤                |
| `telemetry_enabled`                | `true`                     |   否 | 布尔开关                          | 重启进程 | 匿名负载遥测采集                  |
| `telemetry_retention_days`         | `90`                       |   否 | `1–3650` 的整数                   | 重启进程 | 负载遥测按小时数据保留            |
| `oidc_issuer_url`                  | 空                         |   否 | 单值 URL                          | 下一请求 | OIDC issuer                       |
| `oidc_client_id`                   | 空                         |   否 | 单值文本                          | 下一请求 | OIDC confidential client ID       |
| `oidc_client_secret`               | 空                         |   是 | 单值密码文本                      | 下一请求 | OIDC client secret                |
| `oidc_display_name`                | `authentik`                |   否 | 单值文本                          | 下一请求 | 登录按钮名称                      |
| `public_base_url`                  | 空                         |   否 | 单值 URL                          | 下一请求 | 公开 Origin 与 OIDC 回调地址      |

OIDC 仅在 `oidc_issuer_url`、`oidc_client_id`、`oidc_client_secret` 和 `public_base_url` 都非空时启用；两个 URL 不得含凭据、query 或 fragment；保存时接受 HTTP，但非 `--development` 进程在使用时要求 HTTPS，否则 OIDC 不可用。回调地址固定为 `<public_base_url>/api/auth/oidc/callback`。IdP 侧配置和账号规则见 [authentik 单点登录](../operations/authentik.md#litradar-运行设置)。

`cnki_captcha_token`、`provider_proxy_url` 和 `oidc_client_secret` 都是 scalar secret，不是 secret pool。公开 GET/PUT 响应始终返回 `value=""` 和 `secret_items=[]`；已配置时只通过 `has_value=true`、`masked_value="••••"` 表示存在。管理页的空密码框表示保留，非空文本表示加密替换，显式“清除”提交 JSON `null`。数据库值为空时，只有 `cnki_captcha_token` 具有 `LITRADAR_CNKI_CAPTCHA_TOKEN` 单次索引探测回退；`serve` 不读取该环境变量，在线摘要 Provider 只使用数据库密文。代理 URL 没有环境变量或 CLI 回退。

默认 `index_provider_routes` 为：

```json
{
  "ccf_computer_journals": "scholarly",
  "chinese_journals": "cnki",
  "english_journals": "scholarly"
}
```

索引命令只根据目录 stem 选择 Provider。只要选中的目录被路由到 `scholarly`，OpenAlex key、Semantic Scholar key 和 Crossref mailto 三类配置都必须至少有一个非空值；缺失任一类都会在创建索引状态前失败。CSV 本身不含 `source`。

key/mailto 池按逗号、分号或换行拆分，去除空项并按首次出现顺序去重。Crossref 始终使用第一个稳定 mailto；更多 mailto 只是备用配置，不增加、拆分或轮转 Crossref 容量。OpenAlex 和 Semantic Scholar 会使用池中全部合法 key，并按各 key 自己的相位、剩余额度、冷却和认证状态选择 slot；这不是忽略健康状态的逐请求简单轮转。

池中的每个 API key 都必须是为该部署合法签发和允许使用的凭据；不要为了规避 Provider 限流、许可或身份规则而创建或收集额外 key。

### Provider 托管代理

`provider_proxy_url` 是所有内置 Provider 共用的唯一代理 authority；`provider_proxy_policy` 决定每个逻辑 Provider 是否使用它。系统不接受每个 Provider 单独的 URL。URL 只允许以下四种 scheme，省略端口时会保存为对应的规范默认端口：

| Scheme    | 默认端口 | 目的地主机解析                                      |
| --------- | -------: | --------------------------------------------------- |
| `http`    |     `80` | 普通 HTTP 代理语义                                  |
| `https`   |    `443` | TLS 连接到代理                                      |
| `socks5`  |   `1080` | LitRadar 所在主机先解析目的地主机，再把 IP 交给代理 |
| `socks5h` |   `1080` | 把原始目的地主机名交给代理解析                      |

值必须是 authority-only URL，例如 `http://proxy.example:8080` 或 `socks5h://user:password@proxy.example:1080`。主机必填，端口不能为 0；只允许空 path 或 `/`，不允许 query、fragment。认证要么完全省略，要么同时提供非空 username 和 password；保留字符应进行 URL percent-encoding。SOCKS5/SOCKS5h 的 username 和 password 解码后各不能超过 255 字节。SOCKS4、SOCKS4a、PAC、SSH、Shadowsocks 及其他 scheme 均被拒绝。

`socks5` 会在应用主机上产生目的地 DNS 查询，可能向本地解析器暴露域名；需要代理侧 DNS 时应选 `socks5h`。这里的区别只影响目的地主机解析，不改变代理 authority 本身的解析和连接。

默认策略为：

```json
{ "cnki": false, "scholarly": false, "zjlib": false }
```

策略必须是 Provider 名称到布尔值的严格 JSON object。缺少的当前或未来 Provider 一律视为 `false`；管理 API 拒绝未知名称、非布尔值和不安全名称，管理页按当前 Provider capability 目录呈现三个独立开关并以稳定名称顺序保存。

URL 与策略在同一个 `PUT /api/admin/runtime-settings` 中按更新后的有效状态一起校验和提交。任何开关为 `true` 时 URL 必须存在；要清除 URL，必须在同一请求中先把所有开关关闭并对 URL 提交 JSON `null`。空白秘密输入保留已有密文，非空输入加密替换。任一字段或同一请求中的其他运行设置无效时，全部设置和必需审计写入一起回滚。

`serve` 在启动时加载一次代理选择；保存后必须重启服务。`index` 在每条新命令启动时读取选择，已经运行的命令和它的 worker 不会热更新。所有受管 Provider 客户端都会明确忽略 `HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY` 等系统代理发现：关闭开关时确定性直连；打开时只走所配置的代理，代理连接或请求失败会返回有界错误，绝不静默直连重试。

开关的网络所有权如下：

- `scholarly`：索引中的 Crossref、OpenAlex 和 Semantic Scholar HTTP；在线摘要只是本地生成受限 DOI/PubMed redirect，不发出 Provider HTTP。
- `cnki`：国内 CNKI 索引和在线摘要的全部 HTTP，以及其 JFBYM captcha 请求；JFBYM 没有独立开关。
- `zjlib`：扫码开始/轮询、会话预热、BFF/Share SSO、搜索和全文下载的全部 HTTP，包括重定向与非重定向客户端。

这两个设置不影响 AI、通知、PushPlus、MCP、浏览器访问返回的 redirect，也不替代 `trusted_proxy_cidrs` 的入站反向代理信任策略。旧 `proxy_pool` 不是兼容别名，提交时仍按未知字段拒绝。代理 URL、userinfo 和 worker bootstrap 都是秘密数据，不得放入参数、环境变量、请求文件、日志、格式化输出、API 响应或运维工单。

### Scholarly 请求预算

| 上游             | 当前合同                                                | LitRadar 安全相位                                                       | 池的含义                                        |
| ---------------- | ------------------------------------------------------- | ----------------------------------------------------------------------- | ----------------------------------------------- |
| Crossref         | polite `10 req/s`、并发 `3`                             | 整个父进程树每 110 ms 一个尝试，约 `9.09 req/s`；最多三个期刊子进程在途 | mailto 是联系身份；数量不乘以容量               |
| OpenAlex         | 所配置免费密钥的历史观测为 `30 req/s/key`，另有每日额度 | 同一进程组内每个密钥每 40 ms 启动一次请求（`25 req/s/key`）             | 各密钥独立计额，不假定拥有 100 RPS 权限         |
| Semantic Scholar | 每 key `1 req/s`                                        | 每个健康 key 跨进程每 1,100 ms 一个相位，约 `0.909 req/s/key`           | 每个合法 key 有独立速率；key 间在周期内均匀错相 |

这些相位协调同一个 `litradar index` 父进程启动的最多三个期刊子进程，不协调另一条命令、另一台主机或其他应用。外部客户端共享同一 key、上游临时降额或窗口实现差异仍可能产生 429；LitRadar 会冷却对应 key 并保留安全证据，不承诺精确 100% 利用率或任何环境下都零限流。

`workers`、`processes` 的默认值、上限和校验时机见 [`index`](cli.md#index)。OpenAlex 的每日预留额度为 `max(total_inflight_capacity * list_cost, actual_process_count * search_cost)`，列表和来源搜索的初始成本分别为 1 和 10 个积分；可信响应显示更高成本时，各类别独立上调。额度未知时，每个密钥、每个进程只允许一次探测。

实际吞吐同时受 Provider 速率、可用在途数、响应延迟和待处理工作量约束，可近似看作 `min(Provider 预算, 在途容量 / 响应延迟, 产生工作速率)`。增加 `workers` 或 `processes` 不能突破每 key 预算；它只在延迟或工作并行度成为瓶颈时提高可达吞吐。

### Provider 路由语法

以下 JSON 设置通过 runtime-setting API 传输时仍是字符串。OpenAPI 分别注册 `IndexProviderRoutes`、`ProviderOrders` 和 `ProviderProxyPolicy`，并在相应值上标注 `contentMediaType`/`contentSchema`；使用者需自行解码字符串，解码后的类型直接引用生成 schema。此描述同时覆盖读取响应和更新请求，持久化值和清除设置所用的 `null` 不变。

`index_provider_routes` 必须是非空 JSON object。key 是不含 `.csv` 的规范目录 stem，value 是已注册的索引 Provider 名称；二者只接受小写 ASCII 名称及内部的 `.`、`_`、`-`。保存时按 key 排序并压缩为规范 JSON。

`article_abstract_provider_orders` 和 `article_fulltext_provider_orders` 使用同一个严格 JSON 结构：

```json
{
  "default": ["scholarly", "cnki"],
  "catalogs": {
    "chinese_journals": ["cnki", "scholarly"],
    "disabled_catalog": []
  }
}
```

- `default` 是没有显式 catalog 条目时的全局有序 fallback。
- `catalogs[stem]` 完整替换默认顺序；不是追加或局部覆盖。
- 缺少条目表示继承；显式 `[]` 表示只禁用该 CSV/同名内容库的动作。
- 顺序中的名称不得重复，且只能使用安全的小写 ASCII 运行时名称；未知 JSON 字段被拒绝。
- 保存时 catalog key 确定性排序并压缩为规范 JSON。

默认摘要配置是 `{"default":["scholarly","cnki"],"catalogs":{}}`，默认全文配置是 `{"default":["zjlib"],"catalogs":{}}`。`scholarly → cnki` 明确表示请求时先尝试 scholarly，失败后再尝试国内 CNKI；它不是索引来源或静态绑定。

管理页调用 `GET /api/admin/provider-catalog`，把 `data/meta/*.csv` 与 `data/index/*.sqlite` 按安全 stem 合并为 catalog 列表，并按 `index_content`、`article_abstract`、`article_full_text` capability 过滤每个控件的候选项。索引 Provider 每个 catalog 单选；摘要页和全文各自支持默认排序、catalog 继承、覆盖与禁用。粒度止于 CSV/database stem，不细化到 CSV 内的期刊。

运行时只尝试已注册且声明相应 capability 的名称。每个顺序独立于 `index_provider_routes`：切换中文索引 Provider 不会自动改变摘要页或全文链路。Provider 的 HTTPS redirect host allowlist 属于代码注册信息，不是管理员配置，也不存入文章或数据库。完整契约见[索引与 Provider 契约](index-provider-contract.md)。

### Origin 语法

`cors_allowed_origins` 和 `mcp_allowed_origins` 的空值都是安全默认值。非空值按逗号拆分并去除首尾空白；空分段会被忽略，重复的有效 Origin 保持原样。

每个非空 Origin 必须是准确的 `http://` 或 `https://` scheme、非空 authority 以及可选显式端口组成的 tuple，例如：

- `https://paper.example`
- `http://localhost:8000`
- `http://[::1]:8000`

不接受 `*` wildcard、裸主机名、非 HTTP(S) scheme、user-info、尾随 `/` 或其他 path、query、fragment。`cors_allowed_origins` 也不接受 `null`；`mcp_allowed_origins` 额外保留精确字面量 `null`，用于现有 opaque Origin MCP 客户端兼容。

管理员提交包含无效 Origin 的 `PUT /api/admin/runtime-settings` 时，API 在写入前返回 `400`，同一请求中的其他字段也不会保存。有效设置不会热加载，在下次 `litradar serve` 启动时生效。

`mcp_allowed_hosts` 的每个非空项还必须是合法 HTTP Header 值，不能包含换行等控制字符。`secure_cookies` 接受大小写不敏感的 `true/false`、`1/0`、`yes/no`、`on/off`，保存时规范化为 `true` 或 `false`；其他值返回 `400`。这两项与 Origin、日志格式和日志 filter 一样，在写入与启动时使用完全相同的 parser，失败的多字段更新整体回滚。

`trusted_proxy_cidrs` 接受逗号分隔的 IPv4/IPv6 CIDR 或单个 IP；单个 IP 规范化为 `/32` 或 `/128`，网络地址会清除 host bits，并按首次出现顺序去重。默认空列表表示任何直连 peer 都不能通过 `Forwarded` 或 `X-Forwarded-For` 改写客户端来源。只有直连 peer 命中该列表时，API 才从右向左跳过可信代理并使用第一个不可信地址；标准 `Forwarded` 存在时不会再回退到 `X-Forwarded-For`。无效链按直连代理地址共享限流，不按攻击者提交的值分桶。

`auth_rate_limit_policy` 使用 `deny_unknown_fields` 的严格 JSON。默认值为：

```json
{
  "login_ip": { "capacity": 30, "refill_tokens": 1, "refill_seconds": 1 },
  "username": { "capacity": 5, "refill_tokens": 1, "refill_seconds": 60 },
  "register_ip": { "capacity": 5, "refill_tokens": 1, "refill_seconds": 60 },
  "global_login": {
    "capacity": 1000,
    "refill_tokens": 100,
    "refill_seconds": 1
  },
  "global_register": {
    "capacity": 250,
    "refill_tokens": 25,
    "refill_seconds": 1
  },
  "ip_key_limit": 8192,
  "username_key_limit": 4096
}
```

所有 capacity/refill/key limit 必须为正并处于固定上限内；全局熔断器的容量必须大于每个前置桶，补充速率不得低于前置桶，防止管理员配置重新引入单一来源耗尽全局额度的问题。该字段规范化为紧凑 JSON；未知、缺失、零值、越界或不满足层级关系时返回 `400`，同一更新整体回滚。

`audit_retention_days` 只接受 `1–3650` 的十进制整数，默认 `180`。统一服务启动后立即执行一次保留检查，之后每 24 小时检查一次；数据库中的持久 maintenance 标记让共享认证库的多个实例在任意 24 小时窗口内最多有一个实例执行删除。每次事务最多删除 10,000 条过期记录；仍有积压时每 60 秒继续下一批，清空后恢复每 24 小时检查。设置更新在下一次检查读取，不需要重启。

`ai_allowed_base_urls` 只接受准确 HTTPS base URL；拒绝 HTTP、凭据、query、fragment 和端口 0，并统一补全 path 尾随 `/`、按首次出现顺序去重。默认空列表会禁用所有 AI 出站请求。普通用户从 `GET /api/tracking/ai-endpoints` 返回的目录中选择，保存时 API 和 storage 事务都会再次做准确成员校验；worker 在每次实际 AI 请求前重新读取当前目录，因此删除目录项会阻止后续尝试继续使用旧配置。

数据库中的无效 Origin 行（例如库外修改）会让应用在绑定端口前以明确配置错误拒绝启动，不会忽略、自动删除或降级该策略；应在维护窗口恢复可验证备份或纠正该非秘密行，再重新启动。

## 读取和更新语义

数据库没有行时使用上表默认值；除 `cnki_captcha_token` 的单次 `litradar index` 探测例外外，不回退到环境变量。`litradar serve` 始终忽略该回退，因此清空数据库 token 并重启后不会继续使用残留环境值。该探测值不改变管理 API 返回的 `source=default`，也不会写回数据库。`next_request` 字段由匹配 API 动作按请求读取，`next_command` 字段在下一条相关命令构造 Provider 前读取，`restart_required` 字段在进程启动时读取。每次响应都返回实际 `source=default|database` 和可选 `updated_at`。

秘密字段响应：

- `value=""`
- `has_value=true|false`
- `masked_value="••••"` 或空字符串
- 秘密池提供逐项 `secret_items=[{reference, masked_value}]`；scalar secret 和非秘密设置返回空数组

秘密池的逐项 `masked_value` 保留前 5 个字符并把其余字符替换为等量 `*`；长度不超过 5 的异常值全部掩码。`reference` 是字段绑定的不透明删除引用，不是完整密钥，也不是 `runtime_settings.value` 中的持久密文。前端用 `secret_items.length` 展示已保存数量，用掩码区分条目。

`PUT` 的 `values` map：

- 秘密字段缺省或空白：保留
- 秘密字段 `null`：清除
- 秘密字段非空：加密替换
- 非秘密字段：保存规范化文本，不接受 `null`
- 未知字段：`400`

`PUT` 的可选 `secret_pool_updates` map：

- map key 必须是受管的秘密池字段
- `add` 接受新增明文数组；每项仍按池分隔符拆分、去空和去重
- `remove` 接受当前 `secret_items.reference` 数组，并按引用解出的完整值精确删除
- 损坏、跨字段或已失效引用返回 `400`，同一请求的其他修改不会提交
- 同一字段同时出现时，先应用 `values` 的保留、替换或清除，再执行 `remove` 和 `add`

前端单项删除只提交引用，不回传掩码或保留项。清除整个池仍使用 `values[field]=null`。最终完整池继续作为一个 `litradarenc:v1:` 认证密文存入原 `runtime_settings` 行，不增加数据库表或明文列。

## 通用库与上游 HTTP 行为

OIDC 使用 go-oidc，Provider metadata 中的 `jwks_uri` 必须能从服务端访问。计划任务使用 robfig/cron 的五字段语法，规则见 [API 参考](api.md#管理接口)。

来源 HTTP 的重定向、方法转换、Cookie 和 gzip 解压由 `net/http` 负责：只跟随 301/302/303/307/308；空 Location 不重发请求，畸形 Location 或非法 URL 转义返回请求错误；反斜杠按 Go URL 规则转义；响应 URL 的 fragment 保留在元数据中、不进入请求行，禁止 fragment 的 Provider（如 Zjlib）在发送下一跳前拒绝该 URL；显式 Referer 按标准客户端规则保留，敏感认证头在跨域重定向时由标准客户端限制。各来源的目标域、路径和跳数限制在发送下一跳前检查；CFP 每条重定向链共享一次 acquisition 的 20 秒上限与 source deadline，获取失败的重试从链头开始，部分 body 不重试。投递出站禁止重定向和压缩。

gzip 使用 Go 的自动协商并解压全部 member，解压后检查响应大小上限。Retry-After 接受无符号整数秒或 `http.ParseTime` 支持的 HTTP 日期，超长延迟有安全上限。

URL query／form 编码使用 `net/url`，保留 pair 与重复 key 的顺序（`*` 编码为 `%2A`，`~` 不编码）。DOI 路径使用 `url.PathEscape` 并保留路径分隔符。代理凭据和 CNKI 查询中的非法百分号转义被拒绝；Zjlib 的 query 采用 `url.ParseQuery`，值中的裸分号需编码为 `%3B`。损坏的查询不能绕过日志脱敏。

## `litradar serve`

优先级和启动顺序：

1. CLI 解析 `host`、`port`、`project-root`、调度间隔、密钥文件和 Secure Cookie 启动门。
2. 迁移 `auth.sqlite` 并预检、迁移现有内容索引库，规则见[数据库版本](database.md#连接和版本)。
3. 若打包的 Meta bundle 存在 manifest，准备持久 Meta 目录。
4. 用密钥验证数据库秘密。
5. 加载全局运行设置并构造受管 Provider 代理选择。
6. 应用 Provider 代理、CORS、MCP、Cookie、可信代理和认证限流策略。
7. 若启用 `--require-secure-cookies` 但设置仍为 `false`，拒绝启动。
8. 构造服务端基础安全响应策略；服务端不读取或托管 UI 构建。
9. 绑定监听端口并并发启动 HTTP 与立即执行的调度 tick。

调度间隔、`--development` 约束和失败语义见 [`serve`](cli.md#serve)。根目录 `pnpm dev` 通过 `scripts/dev.mjs` 同时管理 Go 服务端与 Vite。

`--require-secure-cookies` 同时选择 hardened HTTPS 响应模式：在 Secure Cookie 启动门通过后，应用为所有响应增加 `Strict-Transport-Security: max-age=31536000`。未传该参数的 loopback HTTP 模式不发送 HSTS。服务端与客户端分别设置安全响应头；客户端 Vite 构建无需内联脚本哈希。

## 索引进程

`litradar index` 的一次运行参数由 CLI 决定；scholarly transport 的 key/mailto 和当前 Provider 代理选择从全局运行设置读取：

普通索引先迁移认证库并验证现有内容库，再执行固定 manifest 发现和可选的官方 Meta 准备，然后验证部署密钥、读取运行设置、校验规范目录，并按 `index_provider_routes` 构造 Provider。内部索引 worker 不重复准备。准备只管理 manifest 声明的持久文件，不替代目录契约校验。

直接模式把当前逻辑 Provider 的受管代理选择只在内存中交给注册构造。多进程索引把调度数据写入可丢弃 worker request JSON，但该 JSON 不含 `cnki_captcha_token` 或 `provider_proxy_url`。父进程启动 child 后，从 child 环境删除探测变量，并通过现有 stdin 管道发送一次带协议版本和 worker ID 的 bootstrap；只有 `provider_name=cnki` 的 bootstrap 可以携带 captcha token，只有策略中已启用的当前 `provider_name` 可以携带共用代理 URL。worker 在 Provider 构造前验证并消费 bootstrap，随后同一 stdin 流只接收 durable commit ACK。bootstrap/协议失败使用固定错误分类并清理 request 文件和 child 进程；命令行、环境、request JSON、日志和格式化输出都不暴露代理秘密。

- OpenAlex key：请求 `/sources` 和 `/works`
- Semantic Scholar key：`x-api-key` 请求头
- Crossref mailto：只作为 Crossref query 参数；不传给 OpenAlex

## 用户通知配置

AI 凭据和 PushPlus 是用户级设置。每个用户在 `notification_settings` 中保存从全局 `ai_allowed_base_urls` 目录选择的主备 OpenAI 兼容 endpoint、key、model、prompt、PushPlus token 和偏好。

字段数量和长度限制及对应状态码见 [API 参考](api.md#收藏与追踪)。

代码只为 base URL 和 model 提供非秘密默认值；默认 base URL 也必须存在于管理员目录中才可执行。没有全局 AI key 或 PushPlus token。CLI `--ai-model` 可以覆盖 model，但不能补 API key。详见[通知与追踪](../guides/notifications.md)。

`delivery_worker_concurrency` 控制单个 `litradar serve` 实例同时监管的手动投递子进程数，默认 2、范围 `1–16`，保存后重启生效。每个用户由 SQLite 唯一约束限制为一个 queued/active run；该池只决定并行执行量，不替代跨实例 lease、revision 或 dedupe。

## 前端网络边界

Web 客户端通过 Caddy 同源访问 API；开发时 Vite 8000 代理至 Go 8001（`LITRADAR_BACKEND_URL` 可改变后端地址）。原生客户端由用户设置服务端 URL，并通过 Bearer 个人令牌访问；详见[客户端指南](../guides/clients.md)。

Web host 使用 `WAILS_SERVER_HOST`（默认 `0.0.0.0`）、`WAILS_SERVER_PORT`（默认 `8080`）、`LITRADAR_CLIENT_ORIGIN_PATTERNS` 和 `LITRADAR_CLIENT_HSTS_MAX_AGE`。Origin patterns 采用 Wails 的主机匹配语法，不是 CORS 白名单；Web host 不注册业务服务，网关与 host 均拒绝 `/wails/*`。客户端没有数据卷或部署密钥。

对外部署时将 `public_base_url` 设置为用户访问的准确 HTTPS Origin，启用 `secure_cookies=true` 并使用 `--require-secure-cookies` 启动门；`trusted_proxy_cidrs` 仅信任网关的 backend 网络（默认 `172.30.42.0/24`，可用 `LITRADAR_BACKEND_SUBNET` 改为无冲突网段）；`mcp_allowed_hosts` 包含公开域名。设置的保存/重启语义见字段表及[Docker 部署](../operations/docker.md)。

## 日志设置

`log_format` 默认 `json`，只接受精确值 `json` 或 `compact`。部署运行和机器解析使用 JSON Lines；本地交互终端可由管理员显式选择 compact。两者分别使用标准 `slog.JSONHandler` 和 `slog.TextHandler`；JSON 元数据字段为 `time`、`level`、`msg`。

`log_filter` 保留 target / span 指令结构，字段模式使用 Go `regexp` 的子串匹配；末尾匹配需显式写 `$`。默认值为：

```text
warn,litradar=info,litradar_api=info,litradar_cli=info,litradar_index=info,litradar_sources=info,litradar_storage=info,litradar_worker=info
```

`off` 完全关闭服务端事件。API 保存时和每个进程启动时都会严格验证；无效 filter、其他 format 值或认证库不可读会让进程在业务工作前失败。日志 bootstrap 只读这两个非秘密字段，不迁移数据库、不解密 key 池，也不读取日志环境变量。完整事件、级别、关联和丢失语义见[日志运维](../operations/logging.md)。

## 负载遥测设置

`telemetry_enabled` 和 `telemetry_retention_days` 属于 `observability` 分组，apply mode 为 `restart_required`，与其他受管设置一样以字符串值存入 `auth.sqlite`，通过同一个运行配置 API 更新：

```http
PUT /api/admin/runtime-settings
Content-Type: application/json

{"values":{"telemetry_enabled":"true","telemetry_retention_days":"90"}}
```

- `telemetry_enabled` 使用布尔解析，默认 `true`。设为 `false` 并重启后，服务既不打开或创建 `data/telemetry.sqlite`，也不再记录观测；已有文件和数据保持原样。管理面板「负载」分区此时显示未启用状态，`GET /api/admin/telemetry` 返回 `enabled=false`。
- `telemetry_retention_days` 只接受 `1–3650` 的整数，默认 `90`。新值在重启后生效，过期小时在下一次每分钟 flush 或关闭时的最终 flush 中删除。保留期短于所选统计时段时，更早的小时没有数据。
- 文件位置固定为 `<project-root>/data/telemetry.sqlite`，`--auth-db` 不会改变它；没有对应的环境变量或命令行参数。

采集范围、隐私边界和存储结构见[数据库参考](database.md#负载遥测库)。

## 路径默认值

以 `project-root` 为根：

| 路径                     | 内容                                    |
| ------------------------ | --------------------------------------- |
| `data/meta`              | 期刊 CSV                                |
| `data/index`             | 索引 SQLite                             |
| `data/index-control`     | 可丢弃 Provider checkpoint/lease SQLite |
| `data/index-work`        | 可丢弃 Crossref 工作集，不进入备份      |
| `data/auth.sqlite`       | 认证和业务库                            |
| `data/push_state`        | `.changes.json` 候选和只读旧状态导入源  |
| `data/folder_push_state` | 只读旧 push 状态导入源                  |
| `data/telemetry.sqlite`  | 可丢弃的匿名负载遥测，不进入备份        |

检索分词在 Go 中完成，不注册 SQLite 分词扩展；不存在选择分词库的运行时路径或环境变量，规则见[数据库参考](database.md#article_search)。

`/usr/share/litradar/meta` 不在 `project-root` 下，只是发布镜像中的固定官方只读 bundle；`data/meta` 才是需要备份和恢复的运行时目录。

## 上游 HTML 字符集

HTML 编码使用 `golang.org/x/net/html/charset.DetermineEncoding` 的 WHATWG 探测：BOM、HTTP charset、前 1024 字节内的 HTML meta、UTF-8 启发式检测，最后回退 windows-1252。未知声明会继续探测。编码确定后严格验证字节序列，并应用 GB18030 修正表、拒绝未定义的 windows-1252 字节。
