# 运行配置参考

Paper Scanner 不使用单一 `.env` 作为配置中心。不同配置来源服务于不同边界，本页说明其职责、默认值和优先级。

## 配置来源

| 来源                                | 范围               | 典型内容                                  |
| ----------------------------------- | ------------------ | ----------------------------------------- |
| CLI 参数                            | 单个进程或单次作业 | 路径、监听地址、并发、超时、dry-run       |
| `data/auth.sqlite.runtime_settings` | 后端全局           | scholarly key 池、CORS、MCP、Cookie       |
| `notification_settings`             | 单个用户           | AI、PushPlus、偏好、投递方式              |
| 前端环境变量                        | 前端构建/运行      | 浏览器 API 地址、服务端 rewrite、监听地址 |
| 部署密钥文件                        | 一个部署           | 认证和解密数据库秘密值                    |
| `RUST_LOG`                          | 一个 Rust 进程     | tracing 过滤                              |

后端不从环境变量读取 scholarly、AI、PushPlus、CORS、MCP、Cookie 或代理凭据。

## 部署密钥文件

`--secret-key-file` 指向恰好 32 字节的原始文件。它不是 `runtime_settings`，不存入 SQLite，也不能由环境变量回退。

需要该文件的公共入口：

- `api`
- `index`
- `notify`
- `push`
- `scheduler`
- `worker`
- `admin secrets migrate/verify`

`admin bootstrap` 和 `admin backup` 不需要密钥。生成、轮换和恢复要求见[安全说明](../operations/security.md)。

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
| `secure_cookies`                | `false`                   |   否 | `ps_session` 的 Secure 标志                   |

存在任意 `source=scholarly` 的 CSV 行时，`index` 在开始前要求 OpenAlex 和 Semantic Scholar key 池都非空。Crossref mailto 建议生产配置，但代码不把它设为启动必填。

key/mailto 池按逗号、分号或换行拆分，去除空项并按首次出现顺序去重；当前实时客户端选择池中的第一个值发起请求。池设计保留多个值，但不表示每次请求都会轮转。

## 读取和更新语义

API、索引器和调度子进程在启动时从 `runtime_settings` 读取有效值；数据库没有行时使用上表默认值，不回退到环境变量。

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

前端单项删除只提交引用，不回传掩码或保留项。清除整个池仍使用 `values[field]=null`。最终完整池继续作为一个 `psenc:v1:` 认证密文存入原 `runtime_settings` 行，不增加数据库表或明文列。

## API 进程

优先级和启动顺序：

1. CLI 解析 `host`、`port`、`project-root`、密钥文件和 Secure Cookie 启动门。
2. 迁移 `auth.sqlite` 和索引库。
3. 用密钥验证数据库秘密。
4. 加载全局运行设置。
5. 应用 CORS、MCP 和 Cookie 策略。
6. 若启用 `--require-secure-cookies` 但设置仍为 `false`，拒绝启动。
7. 绑定监听端口。

## 索引进程

`index` 的一次运行参数由 CLI 决定；scholarly transport 的 key/mailto 从全局运行设置读取：

- OpenAlex key：请求 `/sources` 和 `/works`
- Semantic Scholar key：`x-api-key` 请求头
- Crossref mailto：Crossref query 参数，同时传给 OpenAlex mailto

CNKI overseas 元数据索引不使用这三个设置，也不读取代理配置。

## 用户通知配置

AI 和 PushPlus 是用户级设置，不是全局运行设置。每个用户在 `notification_settings` 中保存主备 OpenAI 兼容 endpoint、key、model、prompt、PushPlus token 和偏好。

代码只为 base URL 和 model 提供非秘密默认值；没有全局 AI key 或 PushPlus token。CLI `--ai-model` 可以覆盖 model，但不能补 API key。详见[通知与追踪](../guides/notifications.md)。

## 前端变量

| 变量                  | 默认值                                                           | 生效位置                                 |
| --------------------- | ---------------------------------------------------------------- | ---------------------------------------- |
| `NEXT_PUBLIC_API_URL` | 空                                                               | 浏览器 API 根地址；空值使用同源 `/api/*` |
| `INTERNAL_API_URL`    | 本地 `http://localhost:8000`；Docker build arg `http://api:8000` | Next.js 服务端 rewrite                   |
| `HOSTNAME`            | 运行环境决定；Compose 为 `0.0.0.0`                               | standalone 服务监听                      |

本地同源开发通常不设置 `NEXT_PUBLIC_API_URL`。浏览器跨源直连时，后端 `cors_allowed_origins` 必须包含前端 Origin。

## `RUST_LOG`

`RUST_LOG` 是 tracing-subscriber 的标准过滤器，不承载业务配置：

```bash
RUST_LOG=error cargo run --bin api -- \
  --secret-key-file secrets/paper-scanner.key
```

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
