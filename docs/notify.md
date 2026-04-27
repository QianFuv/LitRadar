# 通知与追踪推送

本文档覆盖当前仓库中与“新增文章后续分发”相关的全部链路，包括：

- `uv run notify`：PushPlus 通知
- `uv run push`：追踪文件夹推送
- `/api/tracking/push-weekly`：面向单个用户的即时推送

与旧版本相比，当前实现已经从“静态订阅文件”迁移到“数据库中的用户通知设置”，并支持 OpenAI 兼容模型配置。

## 总体设计

当前系统存在两条并行分发链路：

1. **PushPlus 通知链路**
   - 入口：`uv run notify`
   - 处理对象：`delivery_method = "pushplus"` 的用户
   - 输出：PushPlus 消息

2. **追踪文件夹链路**
   - 入口：`uv run push`
   - 处理对象：`delivery_method = "folder"` 且已配置追踪文件夹的用户
   - 输出：将文章写入用户收藏库中的追踪文件夹

此外，前端用户还可以主动调用：

- `POST /api/tracking/push-weekly`

这条接口会读取最近的变更清单，把适合当前用户的文章推入自己的追踪文件夹。

## 数据来源

通知与追踪推送都不是直接按“最近 7 天文章日期”扫描数据库，而是依赖索引增量更新时生成的变更清单：

- `data/push_state/<db>.changes.json`

该文件由 `uv run index --update` 生成，核心内容包括：

- `changed_issue_keys`
- `changed_inpress_journal_ids`
- `notifiable_article_ids`
- `backfill_article_ids`
- `summary`

只有被归入 `notifiable_article_ids` 的新增文章才会进入“每周更新 / 通知 / 追踪推送”主链路。

## 用户配置来源

当前有效订阅源为 `data/auth.sqlite` 中的 `notification_settings` 表。
代码从数据库加载用户偏好，不再读取旧的订阅 JSON。

字段包括：

| 字段 | 说明 |
| --- | --- |
| `keywords` | 关键词偏好 |
| `directions` | 研究方向偏好 |
| `delivery_method` | 当前只支持 `folder` 或 `pushplus` |
| `pushplus_token` | PushPlus 令牌 |
| `pushplus_template` | 推送模板 |
| `pushplus_topic` | 可选 topic |
| `pushplus_channel` | 可选渠道，默认 `wechat` |
| `ai_base_url` | 用户级 OpenAI 兼容接口地址 |
| `ai_api_key` | 用户级 API Key |
| `ai_model` | 用户级模型名 |
| `ai_system_prompt` | 用户级自定义系统提示词 |
| `enabled` | 是否启用 |

## 运行时配置

### 推荐的环境变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `NOTIFY_AI_BASE_URL` | `https://api.siliconflow.cn/v1` | 默认 OpenAI 兼容基地址 |
| `NOTIFY_AI_API_KEY` | 空 | 默认 AI Key |
| `NOTIFY_AI_MODEL` | `deepseek-ai/DeepSeek-V3` | 默认模型名 |
| `NOTIFY_AI_SYSTEM_PROMPT` | 空 | 默认系统提示词 |
| `NOTIFY_MAX_CANDIDATES` | `120` | 送入模型的候选上限 |
| `NOTIFY_TEMPERATURE` | `0.2` | 模型温度 |
| `NOTIFY_PUSHPLUS_CHANNEL` | `wechat` | PushPlus 默认渠道 |
| `NOTIFY_PUSHPLUS_TEMPLATE` | `markdown` | PushPlus 默认模板 |
| `NOTIFY_PUSHPLUS_TOPIC` | 空 | PushPlus 默认 topic |
| `NOTIFY_PUSHPLUS_OPTION` | 空 | PushPlus 默认 option |

通知链路现在只识别上述 `NOTIFY_AI_*` 变量，不再解析旧的 OpenAI / SiliconFlow 别名。

## AI 选择逻辑

当前 AI 选择器为 `OpenAICompatibleSelector`，使用 OpenAI Python SDK 调用兼容聊天补全接口。

### 选择原则

模型提示词的核心要求：

- 先按研究方向做第一轮过滤
- 再在方向命中的候选中按关键词排序
- 只根据文章内容相关性与质量判断
- 不得凭空编造文章 ID

### AI 不可用时的跳过策略

如果满足以下任一条件，系统会跳过对应用户的本次推送：

- 用户未配置关键词与研究方向
- 用户配置了偏好，但 AI Key 或模型名为空
- AI 请求异常

跳过后的行为：

- `notify`：记录该 PushPlus 订阅用户为 skipped，不发送消息
- `push`：记录该追踪文件夹订阅用户为 skipped，不写入收藏
- `/api/tracking/push-weekly`：返回 completed 状态与跳过原因，不执行全量推送

## `uv run notify`

### 作用

从变更清单或快照差异中筛选候选文章，对 PushPlus 订阅用户逐个做去重、AI 选择和消息发送。

### 处理对象

仅处理：

- `delivery_method = "pushplus"`
- `pushplus_token` 非空
- `enabled = true` 的数据库订阅用户

### 命令行参数

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `--db` | 自动检测 | `data/index/` 下的数据库名 |
| `--state-dir` | `data/push_state` | 状态文件目录 |
| `--changes-file` | 空 | 指定增量更新变更清单 |
| `--ai-model` | 空 | 覆盖默认 OpenAI 兼容模型名 |
| `--max-candidates` | `0` | 0 表示使用全局默认值 |
| `--timeout` | `60` | HTTP 超时秒数 |
| `--retries` | `3` | AI 与 PushPlus 重试次数 |
| `--dedupe-retention-days` | `60` | 去重记录保留天数 |
| `--dry-run` | `false` | 不真正发送消息 |

### 运行流程

1. 解析目标数据库
2. 读取状态文件 `data/push_state/<db>.json`
3. 如果提供 `--changes-file`，按变更清单运行；否则按前后快照差异运行
4. 加载 issue 与 in-press 候选
5. 执行 AI 选择；无可用偏好或 AI 配置时跳过对应订阅用户
6. 构造 Markdown 内容并发送 PushPlus
7. 更新 `delivery_dedupe` 与运行状态

## `uv run push`

### 作用

与 `notify` 共用候选选择与 AI 筛选逻辑，但最终不发送 PushPlus，而是把文章写入用户追踪文件夹。

### 处理对象

仅处理：

- `delivery_method = "folder"`
- 已配置 `tracking_folder_id`
- `enabled = true`

### 命令行参数

与 `notify` 基本一致，只是默认状态目录变为：

- `data/folder_push_state`

### 运行结果

- 正常模式：调用 `bulk_add_favorites(...)` 写入用户追踪文件夹
- `--dry-run`：只输出选择结果，不写库

## `/api/tracking/push-weekly`

这是面向当前登录用户的手动后台任务，与 `uv run push` 不同之处在于：

- 只处理当前用户
- 读取最新的 `data/push_state/*.changes.json`
- `POST /api/tracking/push-weekly` 会立即返回当前任务状态
- 可通过 `GET /api/tracking/push-weekly/status` 轮询状态
- `delivery_method = "folder"` 时写入追踪文件夹
- `delivery_method = "pushplus"` 时发送 PushPlus；开启 `sync_to_tracking_folder` 后才同步写入追踪文件夹

返回结果里常见字段：

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

### PushPlus 通知状态

- 路径：`data/push_state/<db>.json`

### 追踪文件夹状态

- 路径：`data/folder_push_state/<db>.json`

状态文件包含：

- `status`
- `updated_at`
- `last_completed_run_at`
- `snapshot`
- `run`
- `delivery_dedupe`

其中 `run` 内部通常会记录：

- `run_id`
- `pending_issue_keys`
- `pending_inpress_keys`
- `done_issue_keys`
- `done_inpress_keys`
- `delivered_article_ids`
- `user_results`
- `errors`

## 每周更新接口与通知链路的关系

`GET /api/weekly-updates` 与通知链路共享同一组变更清单，但用途不同：

- `weekly-updates`：面向前端展示，把新增文章按数据库和期刊聚合
- `notify` / `push`：面向分发，把新增文章送入 PushPlus 或追踪文件夹

因此如果你发现：

- 前端“每周更新”为空
- `push-weekly` 无可推送文章
- `notify` 提示没有更新

优先检查的应该是：

- `data/push_state/*.changes.json` 是否存在
- 清单中的 `notifiable_article_ids` 是否为空

## 说明

当前通知与追踪推送链路以数据库订阅配置和 `data/push_state/` 下的状态文件为准。

这些文件仅用于历史兼容和示例说明，当前实际订阅源是 `data/auth.sqlite`。
