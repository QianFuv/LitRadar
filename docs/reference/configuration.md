# 运行配置参考

LitRadar 不使用单一 `.env` 作为配置中心。不同配置来源服务于不同边界，本页说明其职责、默认值和优先级。

## 配置来源

| 来源                                | 范围              | 典型内容                                      |
| ----------------------------------- | ----------------- | --------------------------------------------- |
| CLI 参数                            | 一个子命令调用    | 路径、监听地址、调度间隔、并发、超时、dry-run |
| `data/auth.sqlite.runtime_settings` | 后端全局          | scholarly key 池、CORS、MCP、Cookie           |
| `notification_settings`             | 单个用户          | AI、PushPlus、偏好、投递方式                  |
| 前端环境变量                        | 前端构建/本地开发 | 浏览器 API 地址、开发 rewrite 目标            |
| 部署密钥文件                        | 一个部署          | 认证和解密数据库秘密值                        |
| `LITRADAR_BUNDLED_META_DIR`         | 发布镜像打包      | 不可变官方 Meta bundle 的内部路径             |
| `LITRADAR_LOG_FORMAT`               | 一个 Rust 进程    | `json` 或 `compact` 日志格式                  |
| `LITRADAR_LOG_FILTER`               | 一个 Rust 进程    | 严格的 tracing filter                         |

后端不从环境变量读取 scholarly、AI、PushPlus、CORS、MCP、Cookie 或代理凭据。`LITRADAR_BUNDLED_META_DIR` 只连接镜像资产与启动准备，不承载这些业务设置。

## 部署密钥文件

`--secret-key-file` 指向恰好 32 字节的原始文件。它不是 `runtime_settings`，不存入 SQLite，也不能由环境变量回退。

需要该文件的公共入口：

- `litradar serve`
- `litradar index`
- `litradar notify`
- `litradar push`
- `litradar scheduler`
- `litradar admin secrets migrate/verify`

`litradar admin bootstrap`、`litradar admin backup` 和 `litradar openapi` 不需要密钥。生成、轮换和恢复要求见[安全说明](../operations/security.md)。

## 官方 Meta 打包路径

发布 Docker 镜像设置 `LITRADAR_BUNDLED_META_DIR=/usr/share/litradar/meta`。该只读目录含 `bundle-manifest.json` 和官方 CSV；持久副本始终位于 `<project-root>/data/meta`。这是 Dockerfile 与应用之间的打包契约，不是 `runtime_settings`、秘密、普通运维覆盖项或 CLI 参数。不要把它指向持久目录。

设置该变量后，`serve` 和普通 `index` 会在认证库迁移后验证整个 bundle，再按 manifest hash 创建、接管或升级官方文件。自定义的同名文件和 manifest 外文件保持不变；结果产生 `storage.managed_meta.prepared` 事件。bundle 缺失、格式/hash 非法、持久目标不是普通目录/文件或检测到版本降级时，命令在后续工作前失败。

本地构建默认不设置该变量，因此不执行受管准备，也不会要求 `/usr/share/litradar/meta` 存在。此时缺失或空的 `<project-root>/data/meta` 继续沿用索引命令原有的无输入行为。

## 全局运行设置

管理员通过 `GET/PUT /api/admin/runtime-settings` 或前端管理页维护以下七项：

| 字段                            | 默认值                    | 秘密 | 使用者                                        |
| ------------------------------- | ------------------------- | ---: | --------------------------------------------- |
| `openalex_api_key_pool`         | 空                        |   是 | scholarly 索引                                |
| `semantic_scholar_api_key_pool` | 空                        |   是 | scholarly 索引                                |
| `crossref_mailto_pool`          | 空                        |   否 | Crossref 联系邮箱；OpenAlex 请求也复用 mailto |
| `cors_allowed_origins`          | 空                        |   否 | API credentialed CORS                         |
| `mcp_allowed_hosts`             | `localhost,127.0.0.1,::1` |   否 | MCP Host 白名单                               |
| `mcp_allowed_origins`           | 空                        |   否 | 浏览器 MCP Origin 白名单                      |
| `secure_cookies`                | `false`                   |   否 | `litradar_session` 的 Secure 标志             |

存在任意 `source=scholarly` 的 CSV 行时，`litradar index` 在开始前要求 OpenAlex 和 Semantic Scholar key 池都非空。Crossref mailto 建议生产配置，但代码不把它设为启动必填。

key/mailto 池按逗号、分号或换行拆分，去除空项并按首次出现顺序去重；当前实时客户端选择池中的第一个值发起请求。池设计保留多个值，但不表示每次请求都会轮转。

### Origin 语法

`cors_allowed_origins` 和 `mcp_allowed_origins` 的空值都是安全默认值。非空值按逗号拆分并去除首尾空白；空分段会被忽略，重复的有效 Origin 保持原样。

每个非空 Origin 必须是准确的 `http://` 或 `https://` scheme、非空 authority 以及可选显式端口组成的 tuple，例如：

- `https://paper.example`
- `http://localhost:8000`
- `http://[::1]:8000`

不接受 `*` wildcard、裸主机名、非 HTTP(S) scheme、user-info、尾随 `/` 或其他 path、query、fragment。`cors_allowed_origins` 也不接受 `null`；`mcp_allowed_origins` 额外保留精确字面量 `null`，用于现有 opaque Origin MCP 客户端兼容。

管理员提交包含无效 Origin 的 `PUT /api/admin/runtime-settings` 时，API 在写入前返回 `400`，同一请求中的其他字段也不会保存。有效设置不会热加载，在下次 `litradar serve` 启动时生效。

旧版本或库外修改留下的无效 Origin 行会让应用在绑定端口前以明确配置错误拒绝启动，不会忽略、自动删除或降级该策略。升级前应通过当前运行的管理 API 修正；若新版本已经无法启动，应在维护窗口恢复可验证备份或纠正该非秘密行，再重新启动。

## 读取和更新语义

统一服务、索引及调度启动的同一二进制子进程在工作前从 `runtime_settings` 读取有效值；数据库没有行时使用上表默认值，不回退到环境变量。

秘密字段响应：

- `value=""`
- `has_value=true|false`
- `masked_value="••••"` 或空字符串
- 秘密池提供逐项 `secret_items=[{reference, masked_value}]`；其他设置返回空数组

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

## `litradar serve`

优先级和启动顺序：

1. CLI 解析 `host`、`port`、`project-root`、调度间隔、密钥文件和 Secure Cookie 启动门。
2. 迁移 `auth.sqlite` 和索引库。
3. 若配置了官方 Meta 打包路径，准备持久 Meta 目录。
4. 用密钥验证数据库秘密。
5. 加载全局运行设置。
6. 应用 CORS、MCP 和 Cookie 策略。
7. 若启用 `--require-secure-cookies` 但设置仍为 `false`，拒绝启动。
8. 绑定监听端口并并发启动 HTTP 与立即执行的调度 tick。

默认调度间隔为 30 秒，可用 `--scheduler-interval-seconds N` 覆盖；N 必须大于 0。任一组件意外失败都会使整个 `serve` 调用失败。

## 索引进程

`litradar index` 的一次运行参数由 CLI 决定；scholarly transport 的 key/mailto 从全局运行设置读取：

普通索引先迁移认证库和现有索引库，再执行可选的官方 Meta 准备，然后验证部署密钥、读取运行设置并进入逐 CSV 预检。内部索引 worker 不重复准备。准备只管理 manifest 声明的持久文件，不绕过或替代期刊身份与数据库投影预检。

- OpenAlex key：请求 `/sources` 和 `/works`
- Semantic Scholar key：`x-api-key` 请求头
- Crossref mailto：Crossref query 参数，同时传给 OpenAlex mailto

CNKI overseas 元数据索引不使用这三个设置，也不读取代理配置。

## 用户通知配置

AI 和 PushPlus 是用户级设置，不是全局运行设置。每个用户在 `notification_settings` 中保存主备 OpenAI 兼容 endpoint、key、model、prompt、PushPlus token 和偏好。

代码只为 base URL 和 model 提供非秘密默认值；没有全局 AI key 或 PushPlus token。CLI `--ai-model` 可以覆盖 model，但不能补 API key。详见[通知与追踪](../guides/notifications.md)。

## 前端变量

| 变量                  | 默认值                  | 生效位置                                                                |
| --------------------- | ----------------------- | ----------------------------------------------------------------------- |
| `NEXT_PUBLIC_API_URL` | 空                      | 前端构建时写入浏览器 API 根地址；空值使用当前 Origin 的同源 `/api/*`    |
| `INTERNAL_API_URL`    | `http://localhost:8001` | 仅供 `next dev` 代理 `/api`、`/mcp`、`/docs` 和 `/openapi.json` 到 Rust |

标准本地开发不设置 `NEXT_PUBLIC_API_URL`：浏览器只访问 Next.js 8000，Rust 内部监听 8001。生产执行静态导出并由 Rust 8000 直接提供，既没有前端运行时环境变量，也不使用 `INTERNAL_API_URL` 或 standalone 监听配置。浏览器跨源直连时，必须在构建前设置 `NEXT_PUBLIC_API_URL`，并让后端 `cors_allowed_origins` 包含前端 Origin。

## 日志变量

`LITRADAR_LOG_FORMAT` 默认 `json`，只接受精确值 `json` 或 `compact`。生产和机器解析使用 JSON Lines；本地交互终端可显式选择 compact。

`LITRADAR_LOG_FILTER` 使用 tracing `EnvFilter` 语法。默认值为：

```text
warn,litradar=info,litradar_api=info,litradar_cli=info,litradar_index=info,litradar_sources=info,litradar_storage=info,litradar_worker=info
```

`off` 完全关闭服务端事件。无效 Unicode、无效 filter 或其他 format 值会让进程在业务工作前失败，不会回退或读取旧的通用 Rust 日志变量。完整事件、级别、关联和丢失语义见[日志运维](../operations/logging.md)。

## 路径默认值

以 `project-root` 为根：

| 路径                     | 内容                   |
| ------------------------ | ---------------------- |
| `data/meta`              | 期刊 CSV               |
| `data/index`             | 索引 SQLite            |
| `data/auth.sqlite`       | 认证和业务库           |
| `data/push_state`        | 变更清单和 notify 状态 |
| `data/folder_push_state` | push 状态              |
| `libs/simple-*`          | 平台 `simple` 扩展     |

`simple` 扩展只按项目根下的内置平台路径发现，不接受环境变量覆盖。

`/usr/share/litradar/meta` 不在 `project-root` 下，只是发布镜像中的官方只读 bundle；`data/meta` 才是需要备份和恢复的运行时目录。
