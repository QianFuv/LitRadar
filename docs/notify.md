# 通知与追踪推送

本文档覆盖当前 Rust 后端中的新增文章分发链路：

- `ps-cli notify dry-run|shadow`：PushPlus 通知计划
- `ps-cli push dry-run|shadow`：追踪文件夹写入计划
- `/api/tracking/push-weekly`：面向当前登录用户的即时推送接口

Python `notify` 和 `push` package scripts 已退休；历史模块只保留给契约测试和 fixture 比对。

## 总体设计

系统存在三条入口：

1. `ps-cli notify dry-run|shadow`
   - 处理 `delivery_method = "pushplus"` 的用户
   - 生成 PushPlus 标题、正文和发送计划
   - `dry-run` 与 `shadow` 不发送外部消息

2. `ps-cli push dry-run|shadow`
   - 处理 `delivery_method = "folder"` 且已配置追踪文件夹的用户
   - 生成收藏写入计划
   - `dry-run` 与 `shadow` 不写入收藏表

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

离线 parity 索引可用 `ps-cli index fixture --manifest ...` 生成兼容变更清单。生产索引库和状态文件可直接放在 `data/` 下供 Rust API/CLI 读取。

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

用户级 AI 配置优先于全局默认值。用户配置备用 endpoint 时，主 endpoint 不可用后会尝试备用配置；主备都不可用时跳过该用户。

## AI 选择逻辑

Rust 推荐逻辑位于 `ps-recommend`。选择原则：

- 先按研究方向做第一轮过滤
- 再在方向命中的候选中按关键词排序
- 只根据文章内容相关性与质量判断
- 不得凭空编造文章 ID

如果用户未配置偏好，或配置了偏好但缺少可用 AI key/model，该用户会被跳过。

## `ps-cli notify`

示例：

```bash
cargo run -p ps-cli -- notify dry-run --auth-db data/auth.sqlite --index-db data/index/utd24.sqlite --db utd24.sqlite
cargo run -p ps-cli -- notify shadow --auth-db data/auth.sqlite --index-db data/index/utd24.sqlite --db utd24.sqlite --changes-file data/push_state/utd24.changes.json
```

参数：

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `--auth-db` | `data/auth.sqlite` | 用户与通知设置数据库 |
| `--index-db` | 必填 | 目标索引 SQLite 文件 |
| `--db` | 必填 | 数据库文件名或显示名 |
| `--state-dir` | `data/push_state` | 状态文件目录 |
| `--changes-file` | 空 | 指定增量变更清单 |
| `--ai-model` | 空 | 覆盖默认模型名 |
| `--max-candidates` | 全局默认值 | 候选文章上限 |
| `--dedupe-retention-days` | `30` | 去重记录保留天数 |

处理对象：

- `delivery_method = "pushplus"`
- `pushplus_token` 非空
- `enabled = true`

## `ps-cli push`

示例：

```bash
cargo run -p ps-cli -- push dry-run --auth-db data/auth.sqlite --index-db data/index/utd24.sqlite --db utd24.sqlite
cargo run -p ps-cli -- push shadow --auth-db data/auth.sqlite --index-db data/index/utd24.sqlite --db utd24.sqlite --changes-file data/push_state/utd24.changes.json
```

参数与 `notify` 一致。处理对象：

- `delivery_method = "folder"`
- 当前用户已设置追踪文件夹
- `enabled = true`

`push` 会生成 planned favorite writes；`dry-run` 和 `shadow` 不写入收藏表。

## `/api/tracking/push-weekly`

该接口面向当前登录用户：

- 读取最新的 `data/push_state/*.changes.json`
- 立即返回任务状态
- 可通过 `GET /api/tracking/push-weekly/status` 轮询
- `delivery_method = "folder"` 时写入追踪文件夹
- `delivery_method = "pushplus"` 时发送 PushPlus
- 开启 `sync_to_tracking_folder` 后，PushPlus 成功后同步写入追踪文件夹

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
