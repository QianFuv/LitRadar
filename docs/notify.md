# 通知与追踪推送

本文档覆盖当前 Rust 后端中的新增文章分发链路：

- `notify`：AI 选择后发送 PushPlus 通知
- `push`：AI 选择后写入追踪文件夹
- `/api/tracking/push-weekly`：面向当前登录用户的即时推送接口

旧 Python `notify` 和 `push` package scripts 已退休；同名 Rust 命令是当前运行入口。

## 总体设计

系统存在三条入口：

1. `notify`
   - 处理 `delivery_method = "pushplus"` 的用户
   - 使用 OpenAI 兼容接口选择文章并生成摘要
   - 生成 PushPlus 标题和正文
   - `--dry-run` 不发送 PushPlus、不写入收藏表

2. `push`
   - 处理 `delivery_method = "folder"` 且已配置追踪文件夹的用户
   - 使用 OpenAI 兼容接口选择文章
   - 写入收藏表
   - `--dry-run` 不写入收藏表

3. `POST /api/tracking/push-weekly`
   - 只处理当前登录用户
   - 可发送 PushPlus 或写入追踪文件夹
   - 可通过 `GET /api/tracking/push-weekly/status` 轮询状态

## 数据来源

通知与追踪推送依赖增量变更清单或状态快照差异：

- `data/push_state/<db>.changes.json`
- `data/push_state/<db>.json`

变更清单的关键字段包括：

- `changed_issue_keys`
- `changed_inpress_journal_ids`
- `notifiable_article_ids`
- `backfill_article_ids`
- `summary`

`index --update` 会生成兼容变更清单。生产索引库和状态文件可直接放在 `data/` 下供 Rust API/CLI 读取。

## 用户配置来源

订阅源为 `data/auth.sqlite` 中的 `notification_settings` 表。字段包括：

| 字段 | 说明 |
| --- | --- |
| `keywords` | 关键词偏好 |
| `directions` | 研究方向偏好 |
| `selected_databases` | 数据库过滤；空列表表示全部数据库 |
| `delivery_method` | `folder` 或 `pushplus` |
| `pushplus_token` | PushPlus 令牌 |
| `pushplus_template` | 推送模板 |
| `pushplus_topic` | 可选 topic |
| `pushplus_channel` | 可选渠道，默认 `wechat` |
| `sync_to_tracking_folder` | PushPlus 推送后是否同步写入追踪文件夹 |
| `ai_base_url` | 用户级 OpenAI 兼容接口地址 |
| `ai_api_key` | 用户级 API key |
| `ai_model` | 用户级模型名 |
| `ai_system_prompt` | 用户级自定义系统提示词 |
| `ai_backup_base_url` | 用户级备用 OpenAI 兼容接口地址 |
| `ai_backup_api_key` | 用户级备用 API key |
| `ai_backup_model` | 用户级备用模型名 |
| `ai_backup_system_prompt` | 用户级备用系统提示词 |
| `ai_retry_attempts` | 每个 AI endpoint 的重试次数 |
| `enabled` | 是否启用 |

## 运行时配置

全局默认值来自环境变量，也可通过管理员后台写入运行时配置。

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `NOTIFY_AI_BASE_URL` | `https://api.siliconflow.cn/v1` | 默认 OpenAI 兼容基地址 |
| `NOTIFY_AI_API_KEY` | 空 | 默认 AI key |
| `NOTIFY_AI_MODEL` | `deepseek-ai/DeepSeek-V3` | 默认模型名 |
| `NOTIFY_AI_SYSTEM_PROMPT` | 空 | 默认系统提示词 |
| `NOTIFY_MAX_CANDIDATES` | `120` | 送入模型的候选上限 |
| `NOTIFY_TEMPERATURE` | `0.2` | 模型温度 |
| `NOTIFY_PUSHPLUS_CHANNEL` | `wechat` | PushPlus 默认渠道 |
| `NOTIFY_PUSHPLUS_TEMPLATE` | `markdown` | PushPlus 默认模板 |
| `NOTIFY_PUSHPLUS_TOPIC` | 空 | PushPlus 默认 topic |
| `NOTIFY_PUSHPLUS_OPTION` | 空 | PushPlus 默认 option |

用户级 AI 配置优先于全局默认值。用户配置备用 endpoint 时，主 endpoint 在重试后仍不可用会尝试备用配置；主备都不可用时跳过该用户。

## AI 选择逻辑

Rust 推荐逻辑位于 `ps-recommend`。选择原则：

- 先按研究方向做第一轮过滤
- 再在方向命中的候选中按关键词排序
- 只根据文章内容相关性与质量判断
- 不得凭空编造文章 ID

如果用户未配置偏好，或配置了偏好但缺少可用 AI key/model，该用户会被跳过。

AI 请求使用 OpenAI 兼容 `/chat/completions` 接口。选择请求会依次尝试 `json_schema`、`json_object` 和无 `response_format` 三种格式；每种格式按 `max(--retries, ai_retry_attempts)` 重试。模型选择结果会经过本地规则过滤，过滤掉不存在的文章 ID 和已在 `delivery_dedupe` 中的文章，并在模型结果不足时用关键词/方向命中的候选补足。最终选中的文章会再请求一次模型生成摘要；摘要失败时保留选择阶段摘要。

## `notify`

示例：

```bash
cargo run --bin notify -- --dry-run
cargo run --bin notify -- --db utd24.sqlite --changes-file data/push_state/utd24.changes.json --no-dry-run
```

参数：

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `--auth-db` | `data/auth.sqlite` | 用户与通知设置数据库 |
| `--index-db` | 空 | 指定单个目标索引 SQLite 文件 |
| `--db` | 空 | 数据库文件名或显示名；省略时处理所有 `data/index/*.sqlite` |
| `--state-dir` | `data/push_state` | 状态文件目录 |
| `--changes-file` | 空 | 指定增量变更清单 |
| `--ai-model` | 空 | 覆盖默认模型名 |
| `--max-candidates` | 全局默认值 | 候选文章上限 |
| `--timeout` | `60` | AI 与 PushPlus 请求超时秒数 |
| `--retries` | `3` | CLI 级重试次数；AI 实际使用 `max(--retries, ai_retry_attempts)` |
| `--dedupe-retention-days` | `60` | 去重记录保留天数 |
| `--dry-run` / `--no-dry-run` | `--no-dry-run` | 是否只演练，不发送 PushPlus |

处理对象：

- `delivery_method = "pushplus"`
- `pushplus_token` 非空
- `enabled = true`

## `push`

示例：

```bash
cargo run --bin push -- --dry-run
cargo run --bin push -- --db utd24.sqlite --changes-file data/push_state/utd24.changes.json --no-dry-run
```

参数与 `notify` 一致，但默认状态目录是 `data/folder_push_state`。处理对象：

- `delivery_method = "folder"`
- 当前用户已设置追踪文件夹
- `enabled = true`

`push` 会写入选中文章到追踪文件夹；`--dry-run` 只返回 planned favorite writes，不写入收藏表。

## PushPlus 发送与去重

`notify --no-dry-run` 会调用 PushPlus `https://www.pushplus.plus/send`。HTTP `429`、`500`、`502`、`503`、`504` 和传输错误会按 `--retries` 重试；PushPlus JSON 响应必须满足 `code = 200`，返回的 `data` 会记录为 `message_id`。

开启 `sync_to_tracking_folder` 后，Rust 命令与旧 Python 行为一致：先写入追踪文件夹，再发送 PushPlus。只有 execute side effects 成功完成后才写入 `delivery_dedupe`；如果 PushPlus 失败，本次 run 标记为 failed，去重记录不会更新。

## `/api/tracking/push-weekly`

该接口面向当前登录用户：

- 读取最新的 `data/push_state/*.changes.json`
- 立即返回任务状态
- 可通过 `GET /api/tracking/push-weekly/status` 轮询
- `delivery_method = "folder"` 时写入追踪文件夹
- `delivery_method = "pushplus"` 时发送 PushPlus
- 开启 `sync_to_tracking_folder` 后，同步写入追踪文件夹并发送 PushPlus

常见返回字段：

- `job_id`
- `status`
- `pushed`
- `selected`
- `summary`
- `total_candidates`
- `message`
- `started_at`
- `finished_at`
- `folder_id`
- `folder_name`

## 状态文件

路径：

- `data/push_state/<db>.json`
- `data/push_state/<db>.changes.json`
- `data/folder_push_state/<db>.json`

`notify` 和 `/api/tracking/push-weekly` 使用 `data/push_state/`；`push` 默认使用 `data/folder_push_state/` 保存追踪文件夹推送状态。变更清单仍统一由 `data/push_state/*.changes.json` 提供。

状态文件通常包含：

- `status`
- `updated_at`
- `last_completed_run_at`
- `snapshot`
- `run`
- `delivery_dedupe`

`run` 内部通常记录：

- `run_id`
- `pending_issue_keys`
- `pending_inpress_keys`
- `done_issue_keys`
- `done_inpress_keys`
- `delivered_article_ids`
- `user_results`
- `errors`

## 排查顺序

如果前端“每周更新”为空、`push-weekly` 无可推送文章，或 CLI 返回 idle：

1. 检查 `data/push_state/*.changes.json` 是否存在
2. 检查 `notifiable_article_ids` 是否为空
3. 检查 `notification_settings` 是否启用对应用户和投递方式
4. 检查用户级或全局 AI 配置是否完整
5. 检查 PushPlus token 是否存在
