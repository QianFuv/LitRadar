# 通知与追踪

本文档说明新增文章如何进入 AI 选择、PushPlus 通知和追踪文件夹。完整命令参数见 [CLI 参考](../reference/cli.md)，设置字段的存储结构见[数据库参考](../reference/database.md)。

## 三个入口

| 入口                             | 用户范围                                     | 投递                         |
| -------------------------------- | -------------------------------------------- | ---------------------------- |
| `litradar notify`                | 所有启用且 `delivery_method=pushplus` 的用户 | PushPlus；可选同步追踪文件夹 |
| `litradar push`                  | 所有启用且 `delivery_method=folder` 的用户   | 追踪文件夹                   |
| `POST /api/tracking/push-weekly` | 当前登录用户                                 | 按该用户的投递方式执行       |

`--dry-run` 不发送 PushPlus，也不写收藏或去重状态；AI 请求仍会执行。

## 输入：变更清单

`litradar index --update` 在 `data/push_state/<db>.changes.json` 写入本次增量变化。分发链路读取的顶层字段包括：

- `changed_issue_keys`
- `changed_inpress_journal_ids`
- `notifiable_article_ids`
- `backfill_article_ids`
- `summary`：仅用于计数和诊断

`summary` 中的明细不是运行输入。没有可用变更清单或状态快照差异时，每周更新、CLI 投递和手动推送可能返回空或 `idle`。

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

`ai_retry_attempts` 的写入范围为 `1..=10`；超出范围的 API 更新会被拒绝。历史或被手工修改的值在读取时归一到该范围，不会触发自动数据库更新。

秘密字段以 `litradarenc:v1:` 密文保存。读取 API 只返回 `has_*` 和固定掩码；更新时：

- 字段缺省或空白字符串：保留现值
- JSON `null`：明确清除
- 非空字符串：替换

不要把掩码作为新值回传。

## AI 配置和选择

投递不读取进程环境变量中的 AI 或 PushPlus 凭据。有效 AI 配置来自用户设置：

- base URL 未填写时使用代码默认 `https://api.siliconflow.cn/v1`
- model 未填写时使用 `deepseek-ai/DeepSeek-V3`，也可由 CLI `--ai-model` 覆盖
- API key 没有可用的全局 fallback，用户必须配置
- 只有用户填写了任一备用字段时才构建备用 endpoint

CLI `--retries` 的范围是 `0..=10`、默认值是 3；用户 `ai_retry_attempts` 的范围是 `1..=10`。两者分别受限后，每个 endpoint 的实际 AI 重试次数仍取两者较大值。该次数分别应用于 `json_schema`、`json_object` 和无 `response_format` 三种兼容形式：值为 N 时，每种形式最多执行一次初始请求和 N 次重试，并按 `1/2/4/8/8...` 秒等待。它不是完整作业时限或所有 endpoint 的请求总数。

请求超时、三种响应格式的顺序和主 endpoint 完整失败后再尝试备用 endpoint 的行为保持不变；`--dry-run` 仍会执行 AI 请求。

模型输出还会经过本地约束：

1. 丢弃不存在的文章 ID。
2. 丢弃当前用户已在 `delivery_dedupe` 中的文章。
3. 若模型结果不足，用标题/摘要命中 keyword 或 direction 的候选补足。
4. 按偏好命中数和模型分数排序。
5. 每次投递最多保留 20 篇。
6. 对最终文章再次请求摘要；失败时保留选择阶段摘要。

## CLI 示例

### PushPlus

```bash
cargo run --bin litradar -- notify \
  --secret-key-file secrets/litradar.key \
  --dry-run

cargo run --bin litradar -- notify \
  --secret-key-file secrets/litradar.key \
  --db utd24.sqlite \
  --changes-file data/push_state/utd24.changes.json \
  --no-dry-run
```

`notify` 默认处理 `data/index/*.sqlite`，状态目录为 `data/push_state`。只有 token 非空、设置启用且投递方式为 `pushplus` 的用户进入执行。

### 追踪文件夹

```bash
cargo run --bin litradar -- push \
  --secret-key-file secrets/litradar.key \
  --dry-run

cargo run --bin litradar -- push \
  --secret-key-file secrets/litradar.key \
  --db utd24.sqlite \
  --changes-file data/push_state/utd24.changes.json \
  --no-dry-run
```

`push` 默认状态目录为 `data/folder_push_state`。目标用户还必须已经设置追踪文件夹。

## 副作用顺序

执行模式先计算全部计划，再按以下顺序产生副作用：

1. 若工作流需要文件夹写入，先添加收藏。
2. `notify` 再发送 PushPlus。
3. 所有当前副作用成功后写入 `delivery_dedupe`。

PushPlus 失败时，本次用户结果失败且不会写入去重记录。若在发送前已经执行了可选文件夹同步，该收藏写入不会被自动回滚；后续重试仍需以最终状态和去重记录为准。

PushPlus 传输使用受限后的 CLI `--retries`，并以相同的 `1/2/4/8/8...` 秒封顶退避对网络错误以及 `429`、`500`、`502`、`503`、`504` 重试。响应 JSON 必须满足 `code=200`，`data` 记录为 message ID。

## 手动推送 API

`POST /api/tracking/push-weekly`：

- 只操作当前认证用户
- 从 `data/push_state/*.changes.json` 读取最新候选
- 立即返回后台 job 状态
- 同一用户已有 running job 时返回该现有状态和 job id，不启动第二份工作
- 每个 `litradar serve` 进程对同一 storage instance 最多接纳 1 个 running manual job
- 另一用户占用该 slot 时，启动请求立即返回通用 `503`；调用方应等待当前 job 进入 completed/failed 后再重试
- 通过 `GET /api/tracking/push-weekly/status` 轮询

该 API 使用与 CLI 相同的选择、投递和状态逻辑，但工作流由当前用户的 `delivery_method` 决定。单槽 admission 只约束当前 `litradar serve` 进程，不提供 `cross-process` 协调；独立调用的投递子命令、计划任务子进程或其他应用实例不受它协调。API 契约见 [API 参考](../reference/api.md)和运行时 OpenAPI。

## 状态文件

| 路径                                | 用途                          |
| ----------------------------------- | ----------------------------- |
| `data/push_state/<db>.changes.json` | 所有入口的增量候选输入        |
| `data/push_state/<db>.json`         | `notify` 和手动 PushPlus 状态 |
| `data/folder_push_state/<db>.json`  | `push` 状态                   |

状态通常包含：

- 顶层 `status`、`updated_at`、`last_completed_run_at`
- `snapshot` 与当前 `run`
- `delivery_dedupe`
- run 的 pending/done keys、已投递文章、用户结果和错误

状态文件由原子写入路径维护，不应手工编辑。

## 与内嵌调度的关系

管理员保存的是类型化 `index`、`notify` 或 `push` job。`litradar serve` 的调度组件按 cron 认领后，通过当前应用可执行路径启动 `litradar index`、`litradar notify` 或 `litradar push` 子进程：

- `index` job 可以在成功后顺序串联 notify/push
- 任一步失败会停止该 job 的后续步骤
- 一个任务失败不阻止同轮其他任务
- `timeout_seconds` 覆盖完整 job 链
- SIGINT/SIGTERM 会终止并等待当前子进程、保存 `cancelled`，且不启动剩余步骤
- dry-run 单次执行使用 `litradar scheduler dry-run-once TASK_ID`

## 排障

按顺序检查：

1. `data/push_state/*.changes.json` 是否存在且 `notifiable_article_ids` 非空。
2. 用户设置是否启用，数据库是否被选中。
3. keyword 或 direction 是否至少有一个非空值。
4. AI key/model 是否可解析，主备 endpoint 是否可访问。
5. `delivery_method=folder` 时是否设置追踪文件夹。
6. `delivery_method=pushplus` 时 token 是否存在。
7. 对应状态目录中的 run/error 是否说明已去重、跳过或传输失败。
8. 调度执行时查看管理后台 scheduler 状态；管理 API 不返回内部 stdout/stderr 摘要。
