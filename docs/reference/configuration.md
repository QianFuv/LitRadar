# 运行配置参考

LitRadar 不使用单一 `.env` 作为配置中心。不同配置来源服务于不同边界，本页说明其职责、默认值和优先级。

## 配置来源

| 来源                                | 范围           | 典型内容                                                  |
| ----------------------------------- | -------------- | --------------------------------------------------------- |
| CLI 参数                            | 一个子命令调用 | 路径、监听地址、调度间隔、并发、超时、dry-run             |
| `data/auth.sqlite.runtime_settings` | 后端全局       | key 池、Provider 路由/顺序、CORS、MCP、Cookie、结构化日志 |
| `notification_settings`             | 单个用户       | AI、PushPlus、偏好、投递方式                              |
| 固定前端/镜像/进程协议              | 构建与运行时   | 同源 API、开发代理、只读 Meta bundle、父子进程日志关联    |
| 部署密钥文件                        | 一个部署       | 认证和解密数据库秘密值                                    |

生产应用不读取 LitRadar 自定义环境变量。旧版的前端 API/开发代理、bundle 路径、日志和父子进程环境覆盖均已删除且没有兼容回退；全局可配置业务值通过管理员前端写入数据库，用户级通知/追踪值通过个人设置中心写入数据库。固定打包/进程协议不属于用户设置，标准测试工具和 OS 进程发现仍保留自己的环境输入边界。

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

发布 Docker 镜像固定把官方 CSV 和 `bundle-manifest.json` 复制到 `/usr/share/litradar/meta`；持久副本始终位于 `<project-root>/data/meta`。应用只检查精确的 `/usr/share/litradar/meta/bundle-manifest.json`，该路径不是 `runtime_settings`、秘密、普通运维覆盖项、环境变量或 CLI 参数。

manifest 存在时，`serve` 和普通 `index` 会在认证库迁移后验证整个 bundle，再按 manifest hash 创建、接管或升级官方文件。自定义的同名文件和 manifest 外文件保持不变；结果产生 `storage.managed_meta.prepared` 事件。bundle 格式/hash 非法、持久目标不是普通目录/文件或检测到版本降级时，命令在后续工作前失败。

本地构建通常没有固定路径或 manifest，发现结果因此为 no-op，不要求 `/usr/share/litradar/meta` 存在。此时运维人员必须自行创建 `<project-root>/data/meta`；目录缺失时索引明确失败，目录存在但没有选中的 CSV 时返回 `skipped`。

## 全局运行设置

管理员通过 `GET/PUT /api/admin/runtime-settings` 或前端管理页维护以下 12 项。响应中的 group、control、apply mode、allowed values 和秘密标记是前端控件的权威元数据；管理页必须逐项呈现，不能用硬编码字段子集代替。

| 字段                               | 默认值                    | 秘密 | 前端控件                          | 生效时机 | 使用者                            |
| ---------------------------------- | ------------------------- | ---: | --------------------------------- | -------- | --------------------------------- |
| `openalex_api_key_pool`            | 空                        |   是 | 可逐项增删的掩码秘密池            | 下一命令 | scholarly 索引                    |
| `semantic_scholar_api_key_pool`    | 空                        |   是 | 可逐项增删的掩码秘密池            | 下一命令 | scholarly 索引                    |
| `crossref_mailto_pool`             | 空                        |   否 | 有序字符串列表                    | 下一命令 | Crossref polite 联系邮箱          |
| `cors_allowed_origins`             | 空                        |   否 | 有序字符串列表                    | 重启进程 | API credentialed CORS             |
| `mcp_allowed_hosts`                | `localhost,127.0.0.1,::1` |   否 | 有序字符串列表                    | 重启进程 | MCP Host 白名单                   |
| `mcp_allowed_origins`              | 空                        |   否 | 有序字符串列表                    | 重启进程 | 浏览器 MCP Origin 白名单          |
| `secure_cookies`                   | `false`                   |   否 | 布尔开关                          | 重启进程 | `litradar_session` 的 Secure 标志 |
| `index_provider_routes`            | 三个官方目录的默认映射    |   否 | 每个 catalog 的能力过滤单选       | 下一命令 | CSV stem 到索引 Provider          |
| `article_abstract_provider_orders` | 见下文                    |   否 | 默认顺序 + catalog 继承/排序/禁用 | 下一请求 | 在线摘要页 fallback               |
| `article_fulltext_provider_orders` | 见下文                    |   否 | 默认顺序 + catalog 继承/排序/禁用 | 下一请求 | 在线全文 fallback                 |
| `log_format`                       | `json`                    |   否 | `json` / `compact` 单选           | 重启进程 | 结构化日志格式                    |
| `log_filter`                       | 见[日志设置](#日志设置)   |   否 | 文本                              | 重启进程 | tracing filter                    |

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

### Scholarly 请求预算

| 上游             | 当前合同                         | LitRadar 安全相位                                                       | 池的含义                                        |
| ---------------- | -------------------------------- | ----------------------------------------------------------------------- | ----------------------------------------------- |
| Crossref         | polite `10 req/s`、并发 `3`      | 整个父进程树每 110 ms 一个尝试，约 `9.09 req/s`；最多三个期刊子进程在途 | mailto 是联系身份；数量不乘以容量               |
| OpenAlex         | 每 key `100 req/s`，另有每日额度 | 每个健康 key 跨进程每 11 ms 一个相位，约 `90.9 req/s/key`               | 每个 key 有独立速率和每日额度                   |
| Semantic Scholar | 每 key `1 req/s`                 | 每个健康 key 跨进程每 1,100 ms 一个相位，约 `0.909 req/s/key`           | 每个合法 key 有独立速率；key 间在周期内均匀错相 |

这些相位协调同一个 `litradar index` 父进程启动的最多三个期刊子进程，不协调另一条命令、另一台主机或其他应用。外部客户端共享同一 key、上游临时降额或窗口实现差异仍可能产生 429；LitRadar 会冷却对应 key 并保留安全证据，不承诺精确 100% 利用率或任何环境下都零限流。

Scholarly 的 `workers` 只控制每个期刊子进程内 OpenAlex DOI 子批的在途上限，范围 `1..=6`；`processes` 范围 `1..=3`。OpenAlex 的全局在途上限为 `workers × processes`。调度器根据响应的剩余额度、reset 和单次 credit cost，为所有可能在途响应保留 `workers × processes × 最大已知单次 cost` 的每日 headroom；额度未知时，每个 key/进程只允许一个探测请求。OpenAlex 请求不再发送 Crossref mailto。

实际吞吐同时受 Provider 速率、可用在途数、响应延迟和待处理工作量约束，可近似看作 `min(Provider 预算, 在途容量 / 响应延迟, 产生工作速率)`。增加 `workers` 或 `processes` 不能突破每 key 预算；它只在延迟或工作并行度成为瓶颈时提高可达吞吐。

### Provider 路由语法

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

默认摘要配置是 `{"default":["scholarly","cnki"],"catalogs":{}}`，默认全文配置是 `{"default":["zjlib_cnki"],"catalogs":{}}`。`scholarly → cnki` 明确表示请求时先尝试 scholarly，失败后再尝试 CNKI；它不是索引来源或静态绑定。

管理页调用 `GET /api/admin/provider-catalog`，把 `data/meta/*.csv` 与 `data/index/*.sqlite` 按安全 stem 合并为 catalog 列表，并按 `index_content`、`article_abstract`、`article_full_text` capability 过滤每个控件的候选项。索引 Provider 每个 catalog 单选；摘要页和全文各自支持默认排序、catalog 继承、覆盖与禁用。粒度止于 CSV/database stem，不细化到 CSV 内的期刊。

运行时只尝试已注册且声明相应 capability 的名称。每个顺序独立于 `index_provider_routes`：切换中文索引 Provider 不会自动改变摘要页或全文链路。Provider 的 HTTPS redirect host allowlist 属于代码注册信息，不是管理员配置，也不存入文章或数据库。完整契约见[索引与 Provider 契约](index-provider-contract.md)。

### Provider 设置迁移

认证库 v6→v7 在单一事务中把旧逗号顺序转换为上述 JSON：

1. 摘要新字段优先使用旧摘要顺序；只有旧摘要行不存在时才使用旧详情顺序。
2. 旧全文顺序迁入全文新字段。
3. 旧顺序成为 `default`，`catalogs` 初始为空；旧空值保留为显式空 default。
4. 成功后删除三个旧字段。重复或非法 Provider 名称会使整个迁移回滚并保留 v6 状态。

公共在线详情 capability 和 `/api/articles/{article_id}/detail` 已删除。前端“文章详情”仍是本地弹窗，用于显示数据库里已经保存的元数据；在线“查看摘要页”使用唯一的摘要 Provider 链路。

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

数据库没有行时使用上表默认值，不回退到环境变量。`next_request` 字段由匹配 API 动作按请求读取，`next_command` 字段在下一条相关命令构造 Provider 前读取，`restart_required` 字段在进程启动时读取。每次响应都返回实际 `source=default|database` 和可选 `updated_at`。

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
2. 迁移 `auth.sqlite`，并验证现有索引库是精确 v4；旧库要求显式重建。
3. 若固定打包路径存在精确 manifest，准备持久 Meta 目录。
4. 用密钥验证数据库秘密。
5. 加载全局运行设置。
6. 应用 CORS、MCP 和 Cookie 策略。
7. 若启用 `--require-secure-cookies` 但设置仍为 `false`，拒绝启动。
8. 绑定监听端口并并发启动 HTTP 与立即执行的调度 tick。

默认调度间隔为 30 秒，可用 `--scheduler-interval-seconds N` 覆盖；N 必须大于 0。任一组件意外失败都会使整个 `serve` 调用失败。

## 索引进程

`litradar index` 的一次运行参数由 CLI 决定；scholarly transport 的 key/mailto 从全局运行设置读取：

普通索引先迁移认证库并验证现有内容库，再执行固定 manifest 发现和可选的官方 Meta 准备，然后验证部署密钥、读取运行设置、校验规范目录，并按 `index_provider_routes` 构造 Provider。内部索引 worker 不重复准备。准备只管理 manifest 声明的持久文件，不替代目录契约校验。

- OpenAlex key：请求 `/sources` 和 `/works`
- Semantic Scholar key：`x-api-key` 请求头
- Crossref mailto：只作为 Crossref query 参数；不传给 OpenAlex

CNKI overseas 元数据索引不使用这三个设置，也不读取代理配置。

## 用户通知配置

AI 和 PushPlus 是用户级设置，不是全局运行设置。每个用户在 `notification_settings` 中保存主备 OpenAI 兼容 endpoint、key、model、prompt、PushPlus token 和偏好。

代码只为 base URL 和 model 提供非秘密默认值；没有全局 AI key 或 PushPlus token。CLI `--ai-model` 可以覆盖 model，但不能补 API key。详见[通知与追踪](../guides/notifications.md)。

## 前端网络边界

前端没有应用专用环境配置。浏览器始终从 `window.location.origin` 生成同源 API URL；`next dev` 通过 Next phase 固定把 `/api`、`/mcp`、`/docs` 和 `/openapi.json` 代理到 `http://127.0.0.1:8001`；其他 phase 始终 `output: 'export'`。生产静态文件和 API 由同一 Rust 监听器提供。

跨源静态前端部署和构建时 API 地址覆盖不再受支持。需要外部域名时，应在同一 Origin 前放置 TLS 反向代理；`cors_allowed_origins` 只服务确有需要的非第一方客户端，不改变第一方浏览器的同源策略。

## 日志设置

`log_format` 默认 `json`，只接受精确值 `json` 或 `compact`。生产和机器解析使用 JSON Lines；本地交互终端可由管理员显式选择 compact。

`log_filter` 使用 tracing `EnvFilter` 语法。默认值为：

```text
warn,litradar=info,litradar_api=info,litradar_cli=info,litradar_index=info,litradar_sources=info,litradar_storage=info,litradar_worker=info
```

`off` 完全关闭服务端事件。API 保存时和每个进程启动时都会严格验证；无效 filter、其他 format 值或认证库不可读会让进程在业务工作前失败。日志 bootstrap 只读这两个非秘密字段，不迁移数据库、不解密 key 池，也不读取通用 Rust 日志变量。完整事件、级别、关联和丢失语义见[日志运维](../operations/logging.md)。

## 路径默认值

以 `project-root` 为根：

| 路径                     | 内容                                    |
| ------------------------ | --------------------------------------- |
| `data/meta`              | 期刊 CSV                                |
| `data/index`             | 索引 SQLite                             |
| `data/index-control`     | 可丢弃 Provider checkpoint/lease SQLite |
| `data/auth.sqlite`       | 认证和业务库                            |
| `data/push_state`        | 变更清单和 notify 状态                  |
| `data/folder_push_state` | push 状态                               |
| `libs/simple-*`          | 平台 `simple` 扩展                      |

`simple` 扩展只按项目根下的内置平台路径发现，不接受环境变量覆盖。

`/usr/share/litradar/meta` 不在 `project-root` 下，只是发布镜像中的固定官方只读 bundle；`data/meta` 才是需要备份和恢复的运行时目录。
