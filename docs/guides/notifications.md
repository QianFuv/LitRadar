# 通知与追踪

本文档说明新增文章如何进入 AI 选择、PushPlus 通知和追踪文件夹。完整命令参数见 [CLI 参考](../reference/cli.md)，设置字段的存储结构见[数据库参考](../reference/database.md)。

## 配置并验证订阅

1. 管理员在运行配置中加入允许访问的 AI HTTPS Endpoint。目录默认为空，未配置时不能执行 AI 投递。
2. 用户在文献追踪设置中填写关键词或研究方向、选择数据库，并配置 AI Endpoint、密钥和模型。
3. 选择投递方式：写入文件夹时先指定追踪文件夹；使用 PushPlus 时填写 token，需要同步收藏时再启用文件夹同步。
4. 保存并启用订阅，确认所选数据库已通过 `index --update` 生成变更清单。
5. 在维护环境先运行对应 CLI 的 `--dry-run`，核对候选与筛选结果，再执行实际投递。手动推送直接产生所选投递方式的副作用，应通过状态接口查看结果。

`--dry-run` 仍会向配置的 AI 服务发送请求并可能产生费用，但不会发送 PushPlus、写收藏或写去重状态。没有候选时不代表配置失败，可按文末的[排障步骤](#排障)定位。

## 三个入口

| 入口                             | 用户范围                                     | 投递                         |
| -------------------------------- | -------------------------------------------- | ---------------------------- |
| `litradar notify`                | 所有启用且 `delivery_method=pushplus` 的用户 | PushPlus；可选同步追踪文件夹 |
| `litradar push`                  | 所有启用且 `delivery_method=folder` 的用户   | 追踪文件夹                   |
| `POST /api/tracking/push-weekly` | 当前登录用户                                 | 按该用户的投递方式执行       |

## 输入：变更清单

`litradar index --update` 在 `data/push_state/<db>.changes.json` 写入本次增量变化。分发链路读取的顶层字段包括：

- `db_name`
- `run_id`
- `changed_issue_keys`
- `changed_inpress_journal_ids`
- `notifiable_article_ids`

每周更新另读取 `generated_at`。`backfill_*` 字段当前恒为空数组，`summary` 仅用于计数和诊断，二者都不是运行输入。没有可用变更清单或状态快照差异时，每周更新、CLI 投递和手动推送可能返回空或 `idle`。

## 用户设置

`data/auth.sqlite.notification_settings` 是唯一订阅源。设置按用途分组：

| 分组     | 字段                                                                                    |
| -------- | --------------------------------------------------------------------------------------- |
| 偏好     | `keywords`、`directions`、`selected_databases`、`enabled`                               |
| 投递     | `delivery_method`、`sync_to_tracking_folder`                                            |
| PushPlus | `pushplus_token`、`pushplus_template`、`pushplus_topic`、`pushplus_channel`             |
| 主 AI    | `ai_base_url`、`ai_api_key`、`ai_model`、`ai_system_prompt`                             |
| 备用 AI  | `ai_backup_base_url`、`ai_backup_api_key`、`ai_backup_model`、`ai_backup_system_prompt` |
| 重试     | `ai_retry_attempts`                                                                     |

`selected_databases=[]` 表示所有数据库。没有非空 keyword/direction、设置未启用、数据库未被选中或没有可用 AI key/model 时，该用户会被跳过。

`delivery_method=folder` 的最终设置必须存在当前追踪文件夹。`delivery_method=pushplus` 必须有非空 token；启用 `sync_to_tracking_folder` 时还必须存在当前追踪文件夹。秘密字段按下文规则保留、替换或清除，最终值以事务开始时的当前值为基础解析。服务在同一个 `BEGIN IMMEDIATE` 中解析最终 token、检查追踪文件夹并写入设置；删除被 folder 投递或 PushPlus 同步依赖的追踪文件夹也使用同一写锁并返回固定 `400`。因此并发的设置保存、secret 清除和文件夹删除无论按何顺序提交，都不会留下缺 token 或缺文件夹的 active subscriber；发现手工损坏的旧状态时投递读取会 fail closed。

普通用户不能输入任意 base URL。设置页只展示管理员运行配置 `ai_allowed_base_urls` 中的准确 HTTPS Endpoint。API 保存时在事务内复核目录，worker 在每次实际 AI 请求前重新读取目录，因此运行中被管理员移除的 Endpoint 不会用于后续尝试。

`ai_retry_attempts` 的写入范围为 `1..=10`；超出范围的 API 更新会被拒绝。历史或被手工修改的值在读取时归一到该范围，不会触发自动数据库更新。

秘密字段以 `litradarenc:v1:` 密文保存。读取 API 只返回 `has_*` 和固定掩码；更新时：

- 字段缺省或空白字符串：保留现值
- JSON `null`：明确清除
- 非空字符串：替换

不要把掩码作为新值回传。

## AI 配置和选择

针对官方 DeepSeek Endpoint（`api.deepseek.com`），当前实现从 `json_object` 开始请求选择和摘要，以适配其 Chat Completions 输出格式；其他 Endpoint 先尝试 `json_schema`。这一格式选择不改变 HTTP 错误重试或备用配置规则。

投递不读取进程环境变量中的 AI 或 PushPlus 凭据。有效 AI 配置来自用户设置：

- base URL 未填写时考虑代码默认 `https://api.siliconflow.cn/v1/`，但只有管理员已把它加入 Endpoint 目录时才可用
- model 未填写时使用 `deepseek-ai/DeepSeek-V3`，也可由 CLI `--ai-model` 覆盖
- API key 没有可用的全局 fallback，用户必须配置
- 只有用户填写了任一备用字段时才构建备用 endpoint

CLI `--retries` 的范围与默认值见 [CLI 参考](../reference/cli.md#notify-和-push)。AI 请求只对连接失败、单次请求超时以及 `429/502/503/504` 重试；`400/401/403`、配置错误和响应结构错误不会网络重试。AI 退避使用 `1/2/4/8/8...` 秒上限内的 full jitter；数值 `Retry-After` 优先使用并封顶 60 秒。只有成功的 2xx 响应明确表现出输出格式不兼容时，才从 `json_schema` 降级到 `json_object` 或普通 JSON。PushPlus 的独立 no-replay 规则见下方“副作用顺序”。

手动任务另有跨主备 Endpoint、格式和摘要请求共享的 8 次 AI HTTP 总预算，以及持久化的 10 分钟绝对 deadline。每次请求 timeout 同时受 120 秒默认值和任务剩余时间限制；这些边界不改变独立 CLI `notify`/`push` 的参数语义。

每次请求都会在总截止时间内通过有界解析器重新解析 DNS，并拒绝 loopback、RFC1918、link-local、unspecified、multicast、IPv6 ULA、NAT64/6to4 和其他特殊用途地址；只允许 HTTPS，禁用环境代理和自动重定向。成功响应必须是未压缩 JSON，最大 2 MiB；非 2xx 响应体不会读取、记录或进入用户可见任务状态。`--dry-run` 仍会执行符合这些边界的 AI 请求。

模型输出还会经过本地约束：

1. 丢弃不存在的文章 ID。
2. 丢弃当前用户已在 `delivery_dedupe` 中的文章。
3. 若模型结果不足，用标题/摘要命中 keyword 或 direction 的候选补足。
4. 按偏好命中数和模型分数排序。
5. 每次投递最多保留 20 篇。
6. 对最终文章再次请求摘要；失败时保留选择阶段摘要。

## CLI 示例

以下 Bash 示例从仓库根目录执行，需要已准备部署密钥、用户设置和所选数据库的变更清单。把 `utd24.sqlite` 与相应清单路径替换为实际数据库；`--no-dry-run` 会执行真实投递。

### PushPlus

```bash
go run ./apps/server notify \
  --secret-key-file secrets/litradar.key \
  --dry-run

go run ./apps/server notify \
  --secret-key-file secrets/litradar.key \
  --db utd24.sqlite \
  --changes-file data/push_state/utd24.changes.json \
  --no-dry-run
```

`notify` 默认处理 `data/index/*.sqlite`，并把运行、进度、去重和 lease 写入 `data/auth.sqlite`。`data/push_state` 只保留 `.changes.json` 输入及不会被自动删除的旧导入源。只有 token 非空、设置启用且投递方式为 `pushplus` 的用户进入执行。

### 追踪文件夹

```bash
go run ./apps/server push \
  --secret-key-file secrets/litradar.key \
  --dry-run

go run ./apps/server push \
  --secret-key-file secrets/litradar.key \
  --db utd24.sqlite \
  --changes-file data/push_state/utd24.changes.json \
  --no-dry-run
```

`push` 与 `notify` 共享认证库中的持久状态表；`data/folder_push_state` 仅可能包含保留的旧导入源。目标用户还必须已经设置追踪文件夹。

## 副作用顺序

执行模式按 subscriber item 逐个推进，并按以下顺序产生副作用：

1. 在外部副作用前，用 SQLite 唯一约束建立每篇文章的 `reserved` dedupe。
2. 若工作流需要文件夹写入，先添加收藏；该写入依赖收藏唯一约束，可在 pre-send 崩溃后幂等重试。
3. `notify` 把 subscriber item 标记为 `sending` 后再发送 PushPlus。
4. 已知成功时，在一个 transaction 中把 subscriber item 与全部 dedupe 落为 `succeeded`/`confirmed`；请求已开始但结果不明确时，同一 transaction 落为 `unknown`。

PushPlus 请求开始后的失败按不确定结果处理：不会回显上游 body，不会释放 dedupe，也不会自动重发。进程在 `claimed` 阶段退出时，过期 owner 的 reservation 会释放并安全重试；在 `sending` 阶段退出时，新 owner 会把 item/dedupe 固定收敛到 `unknown`。若发送前已经执行可选文件夹同步，收藏不会回滚，但其唯一约束确保恢复不会重复创建。

PushPlus 传输只在连接建立明确失败、请求尚未发送时使用受限后的 CLI `--retries` 和 full-jitter。timeout、任何 HTTP 响应、连接后的 transport 错误以及响应解析失败都可能发生在上游已经处理请求之后，因此立即结束本次 `sending` attempt 并落为 `unknown`，不会使用 `Retry-After` 自动重发。响应 JSON 必须满足 `code=200`，`data` 记录为 message ID。

## 手动推送 API

`POST /api/tracking/push-weekly`：

- 只操作当前认证用户
- 从 `data/push_state/*.changes.json` 读取最新候选
- 把 job、绝对 deadline、取消标志和终态写入 `data/auth.sqlite`，以 `202` 立即返回
- 同一用户已有 queued/running job 时返回该现有状态和 job id，不启动第二份工作
- 不同用户进入实例级有界队列，并发上限由 `delivery_worker_concurrency` 控制（见[运行配置](../reference/configuration.md#用户通知配置)）
- 通过 `GET /api/tracking/push-weekly/status` 轮询
- 可通过 `GET /api/tracking/push-weekly/runs/{run_id}` 恢复指定任务，通过 `POST .../{run_id}/cancel` 请求取消；owner 和管理员可访问
- `unknown` 时普通启动仍返回 `409`；只有 owner 检查投递记录后，才可通过 `POST /api/tracking/push-weekly/runs/{run_id}/acknowledge` 显式确认并排入一个新任务

runtime dispatcher 从 SQLite 认领任务，通过隐藏的类型化 `delivery-run` 子命令和完整进程树监管执行。服务重启后 queued 或 lease 过期的任务仍可恢复；取消与 deadline 会先给 cooperative polling 一个短暂窗口，再终止完整进程树。强制终止时若外部副作用可能已经开始，顶层任务固定为 `unknown`，UI 不提供无提示重试。公开状态为 `idle/pending/running/completed/failed/cancelled/timed_out/unknown`，其中 `idle` 表示没有任务。API 契约见 [API 参考](../reference/api.md)和运行时 OpenAPI。

Unknown 确认在一个 `BEGIN IMMEDIATE` 中复核目标属于当前用户、仍是最新手动任务且状态仍为 `unknown`，随后创建一个 queued replacement 并写入固定 schema 的 `manual_push_unknown_acknowledge` 安全审计。并发重复、过期或非 Unknown 确认不会创建第二个任务；管理员也不能代替 owner 确认。确认不会修改旧外层/内层 run、item 或 `unknown`/`confirmed` dedupe，因此不确定文章不会重发，而后续 manifest 中未出现过的新文章仍可正常投递。

## 索引侧 manifest 与 notify 恢复

`index --update` 先把精确 manifest payload、相对目标路径和 outbox cursor 持久化到项目 batch ledger，再原子发布文件并确认 outbox；配置 `--notify` 时，父进程在启动 notify child 前持久化 attempt ID。任一步中止后，默认 resume 重放相同的 manifest 字节，不再次访问已完成的 Provider journal。attempt 只改变外层 scheduled run 身份，文章级 Confirmed/Unknown 去重跨 attempt 保留，因此已确认或不确定的文章不会重发。

child 结果为 `unknown` 或输出不可信时，默认 resume 不再启动 child。审核认证库中的 delivery run、subscriber item 与 dedupe 后，在原 `--resume --update --notify` 命令上增加 `--acknowledge-unknown-notify` 确认并建立新 attempt；存在待完成的已发布 notify manifest 时，`--no-resume` 会失败。handoff 状态判定、attempt 复用和 64 KiB 输出上限见 [CLI 参考](../reference/cli.md#实时恢复与增量同步)。

如果新 batch 的 outbox 为空，而目标已有一个大小受限、可解析且 `db_name` 匹配的 manifest，索引会保留现有文件、返回无新 manifest，并跳过内联 notify；不会用空 payload 覆盖尚待消费的候选。删除项目 batch ledger 会失去这些 phase 与 intent 证明，此后文件与 SQLite 之间按至少一次语义处理。

## 持久状态与旧文件

认证库的 `delivery_checkpoints`、`delivery_runs`、`delivery_run_items`、`delivery_dedupe` 和 `delivery_leases` 表结构见[数据库参考](../reference/database.md#持久投递状态)。这些表通过唯一约束、owner lease 和单调 revision 为多进程投递提供事务边界；外部发送已经开始但结果不明确时使用 `unknown`，不能自动重放。

文件边界如下：

| 路径                                | 用途                                     |
| ----------------------------------- | ---------------------------------------- |
| `data/push_state/<db>.changes.json` | 保持文件形式的增量候选输入               |
| `data/push_state/<db>.json`         | 保留的旧 notify/手动 PushPlus 状态导入源 |
| `data/folder_push_state/<db>.json`  | 保留的旧 push 状态导入源                 |

启动时会先读取并校验全部旧 `<db>.json`，再在一个 transaction 中导入。相同 SHA-256 重复导入会跳过；任何文件损坏都使整批零写入，已导入文件内容变化会拒绝启动。导入不会删除源文件，也不会读取或改写 `.changes.json`。不要手工编辑保留的旧源文件。

## 与内嵌调度的关系

管理员保存的是类型化 `index`、`notify` 或 `push` job。`litradar serve` 的调度组件按 cron 认领后，通过当前应用可执行路径启动 `litradar index`、`litradar notify` 或 `litradar push` 子进程：

- `index` job 可以在成功后顺序串联 notify/push
- 任一步失败会停止该 job 的后续步骤
- 一个任务失败不阻止同轮其他任务
- `timeout_seconds` 覆盖完整 job 链
- SIGINT/SIGTERM 会终止并等待当前子进程、保存 `cancelled`，且不启动剩余步骤
- `litradar scheduler dry-run-once TASK_ID` 只确认任务存在，不执行任何步骤；需要预演投递时直接对 `notify`/`push` 使用 `--dry-run`

## 排障

按顺序检查：

1. `data/push_state/*.changes.json` 是否存在且 `notifiable_article_ids` 非空。
2. 用户设置是否启用，数据库是否被选中。
3. keyword 或 direction 是否至少有一个非空值。
4. AI key/model 是否可解析，主备 endpoint 是否仍在管理员目录且可访问。
5. `delivery_method=folder` 时是否设置追踪文件夹。
6. `delivery_method=pushplus` 时 token 是否存在。
7. 认证库 `delivery_runs`、`delivery_run_items`、`delivery_dedupe` 和 `delivery_leases` 是否显示 busy、skipped、failed 或 unknown；不要修改保留的旧 JSON 来修复运行状态。
8. 调度执行时查看管理后台 scheduler 状态；管理 API 不返回内部 stdout/stderr 摘要。
