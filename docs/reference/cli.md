# CLI 参考

LitRadar 只发布一个可执行文件 `litradar`。本文档是其七个规范子命令、参数和默认值的完整参考。任务流程分别见[开发指南](../guides/development.md)、[Docker 部署](../operations/docker.md)、[通知与追踪](../guides/notifications.md)和[备份与恢复](../operations/backup.md)。

## 调用形式

本地源码：

```bash
cargo run --bin litradar -- <subcommand> <arguments>
```

已安装二进制：

```bash
litradar <subcommand> <arguments>
```

Compose 镜像的入口已经是 `litradar`：

```bash
docker compose run --rm litradar <subcommand> <arguments>
```

顶层 `--help` 只列出：

- `serve`
- `admin`
- `index`
- `notify`
- `push`
- `scheduler`
- `openapi`

每个子命令都接受 `--help` 或 `-h`。未知子命令会写入 stderr 并以非零状态退出。

## 公共路径参数

除 `serve` 和 `openapi` 的特殊边界外，业务子命令共享：

| 参数                  | 默认值                            | 含义                                     |
| --------------------- | --------------------------------- | ---------------------------------------- |
| `--project-root PATH` | 当前工作目录                      | 解析 `data/`、`libs/` 和相对路径的根目录 |
| `--auth-db PATH`      | `<project-root>/data/auth.sqlite` | 显式认证/业务数据库                      |

`serve` 接受 `--project-root`，但不接受 `--auth-db`；它始终使用项目根下的 `data/auth.sqlite`。相对路径按 `project-root` 解析，绝对路径保持不变。

## `serve`

```text
litradar serve --secret-key-file PATH
    [--host HOST]
    [--port PORT]
    [--project-root PATH]
    [--scheduler-interval-seconds N]
    [--require-secure-cookies]
```

| 参数                             | 默认值       | 含义                                                 |
| -------------------------------- | ------------ | ---------------------------------------------------- |
| `--secret-key-file PATH`         | 必填         | 32 字节部署密钥                                      |
| `--host HOST`                    | `127.0.0.1`  | HTTP 监听地址                                        |
| `--port PORT`                    | `8000`       | HTTP TCP 端口                                        |
| `--project-root PATH`            | 当前工作目录 | 数据、静态 Web 和扩展根目录                          |
| `--scheduler-interval-seconds N` | `30`         | 立即执行首个 tick 后的调度间隔；必须大于 0           |
| `--require-secure-cookies`       | 关闭         | 要求数据库 `secure_cookies=true`，否则绑定端口前失败 |

`serve` 是唯一常驻入口。它先准备和迁移存储，再在一个进程中并发运行 HTTP 与内嵌调度。计划任务使用当前 `litradar` 可执行文件启动子命令进程。SIGINT/SIGTERM 会取消活动子进程并保存 `cancelled`；任一运行组件意外失败会关闭另一组件并使进程非零退出。

## `admin`

`admin` 是本机维护入口，不启动 HTTP 或调度循环。

### 初始化管理员

```text
litradar admin bootstrap
    --username NAME
    --password-stdin
    [--project-root PATH]
    [--auth-db PATH]
```

- 只从 stdin 读取一行密码。
- 只在用户表为空时成功。
- 不需要部署密钥。

### 迁移和验证秘密

```text
litradar admin secrets migrate
    --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]

litradar admin secrets verify
    --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]
```

`migrate` 把明文秘密转换为 `litradarenc:v1:`；`verify` 只验证当前密文。操作顺序见[安全说明](../operations/security.md)。

### 轮换部署密钥

```text
litradar admin secrets rotate
    --old-key-file PATH
    --new-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]
```

两个 key 文件都必须是 32 个原始字节。

### 备份

```text
litradar admin backup create
    --output PATH
    [--include-indexes]
    [--include-push-state]
    [--project-root PATH]
    [--auth-db PATH]

litradar admin backup verify
    --backup PATH
    [--project-root PATH]

litradar admin backup restore
    --backup PATH
    --confirm-restore
    [--project-root PATH]
    [--auth-db PATH]
```

备份命令不接收部署密钥。清单格式名固定为 `litradar-backup`；新备份使用 version 2，并始终包含认证库和完整 `data/meta` 普通文件树。`--include-indexes` 和 `--include-push-state` 只选择额外组，后者同时选择 `data/push_state` 和 `data/folder_push_state`。验证和恢复仍接受 version 1；v1 恢复不会修改目标 Meta 目录。精确替换和离线门禁见[备份与恢复](../operations/backup.md)。

## `index`

```text
litradar index --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]
    [--file FILE]
    [--workers N]
    [--processes N]
    [--issue-batch N]
    [--timeout N]
    [--resume | --no-resume]
    [--update | --no-update]
    [--notify | --no-notify]
    [--notify-dry-run | --no-notify-dry-run]
```

| 参数                                       | 默认值   | 含义                                 |
| ------------------------------------------ | -------- | ------------------------------------ |
| `--secret-key-file PATH`                   | 必填     | 解密 scholarly 运行配置              |
| `--file FILE`、`-f FILE`                   | 全部 CSV | 只处理 `data/meta/` 下的一个文件     |
| `--workers N`、`-w N`                      | `8`      | 每个期刊子进程内的 CNKI 文章详情并发 |
| `--processes N`                            | `1`      | 单个 CSV 的期刊子进程数              |
| `--issue-batch N`                          | `8`      | 每轮合并的 CNKI issue 数             |
| `--timeout N`                              | `20`     | 上游 HTTP 超时秒数                   |
| `--resume` / `--no-resume`                 | 开启     | 是否跳过已完成期刊/年份              |
| `--update` / `--no-update`                 | 关闭     | 是否生成增量变更清单                 |
| `--notify` / `--no-notify`                 | 关闭     | 更新成功后启动 `litradar notify`     |
| `--notify-dry-run` / `--no-notify-dry-run` | 关闭     | 下游 notify 是否 dry-run             |

约束：

- `workers`、`processes`、`issue-batch` 必须至少为 1。
- `--notify` 必须和 `--update` 同时使用。
- 单独传 `--notify-dry-run` 不会启动 notify；它只修改 `--notify` handoff 的模式。
- `--workers` 不扩大 scholarly 请求并发；Semantic Scholar 按 `processes` 做进程感知错峰。
- 多个 CSV 仍逐个处理。
- `8/1/8` 是约 100 MiB 索引内存目标下的默认并发。显式提高任一并发参数仍受支持，但可能超过该预算。

索引多进程也通过当前可执行路径启动 `litradar index` 的内部工作请求；不依赖另一个程序名。同步 CLI 命令不创建 Tokio 工作线程池，只有 `serve` 使用固定为 2 个工作线程的小型异步运行时。

命令结果保持原有顶层 `status`、`message` 和 `csvs` 字段，并新增不含密钥的 `effective_concurrency`，记录本次实际使用的 `workers`、`processes` 和 `issue_batch`。每个 CSV 结果使用定长的 `written_article_count`；旧的 `written_article_ids` 列表不再返回。内部索引工作进程同样只返回计数，避免结果大小随文章数量增长。

发布镜像设置 `LITRADAR_BUNDLED_META_DIR=/usr/share/litradar/meta`。普通 `index` 在数据库迁移后、读取密钥和运行设置前准备持久的 `<project-root>/data/meta`，再进入下述期刊预检；内部多进程 worker 请求在准备分支之前返回，因此不会重复准备。报告以 `component=managed_meta` 的结构化 JSON 写入 stderr，不改变上述 stdout JSON。未设置该变量的本地运行不执行受管准备；缺失或空目录仍沿用原有无输入行为。该变量是发布打包契约，不是索引 CLI 选项。

### Meta 期刊预检

每个非空 CSV 在索引或更新前都经过 Meta 期刊预检。显式传入 `--file` 时只检查该文件；未传入时仍按文件名顺序逐个检查和处理，不把不同目录之间重复出现的同一期刊视为冲突。

预检顺序如下：

1. 在创建数据库运行记录前检查 `source`，并确认本文件内每行都能生成唯一稳定期刊 ID。重复项会在错误中同时列出两条期刊标题和身份值；系统不会模糊猜测并自动改写 CSV。
2. 取得该索引库的运行租约并启动心跳后，在一个立即事务中补齐缺失的中性 `journals` 行，并同步 `journal_meta` 的 `source_csv`、`area`、`csv_title`、`csv_issn` 和 `csv_library`。
3. 事务内逐行复验 journal 与上述 CSV 字段。全部匹配并提交后，才会构造本地数据源客户端或启动多进程 worker。

已有 `journals` 行及 `journal_meta.resolved_*` 上游解析字段不会被预检覆盖；缺失 journal 的可用性、文章存在性、排名和电子 ISSN 等 provider 字段保持 `NULL`，等待正常索引解析。输入身份错误在运行记录创建前失败；数据库同步或复验错误会回滚整个目录变更，把已创建的父运行标记为 `failed`，释放本方租约，并保持 worker 启动数为零。之后发生的错误才属于正常上游索引阶段。

该预检是 rate-limit neutral：它只读取 CSV 和本地 SQLite，不额外探测 Crossref、OpenAlex、Semantic Scholar 或 CNKI，因此不能保证上游在随后请求时仍然可用。需要修正歧义身份时，应先审查并更新 `data/meta/*.csv`，再重跑索引。内嵌调度任务最终调用同一个 `litradar index` 路径，自动执行相同预检。

### 实时恢复与增量同步

每个非空 CSV 的实时索引或更新在对应索引库中取得 `index_run_lease`。父运行先以 `running` 持久化，后台每 30 秒续期，租约有效期为 300 秒。同一数据库存在未过期所有者时，新命令会在调用上游或启动 worker 前失败；不要并发重试或手工删除租约。正常错误会写入 `failed` 并释放租约。进程被强制终止时，确认旧进程确实消失，等待租约过期后重跑；下一次命令会把旧父运行标为 `interrupted` 并继续。

`--update` 会在取得租约的同一事务中接管所有旧运行尚未发布的变更事件。worker、上游或清单失败都保留这些事件；只有 changes JSON 已落盘且最终数据库事务成功后才清理。文件发布和 SQLite 提交之间的崩溃可能让同一净变更再次出现，因此该接口保持至少一次投递，消费者必须按既有身份去重。非更新索引不会接管或删除待发布事件。

scholarly 更新不会因为默认 `--resume` 而跳过已完成期刊，而是从上次可信完成时间向前重叠 30 天（`30-day` overlap）。Crossref 请求使用 `from-update-date`，OpenAlex fallback 使用 `from_created_date`；每一页使用相同起始日期。过滤结果为空时保留已有期刊元数据和文章，只在完整页序列成功后推进完成时间。缺少已完成水位、水位无效或位于未来，以及普通非更新索引，都退回完整历史扫描；中断重试仍从旧水位开始。

CNKI 对 HTTP 2xx 正文解码失败使用既有的三次上限和 1/2 秒退避：失败尝试会写入 `index_api_call_stats`，后续成功会标记为 retry。非 2xx 状态在正文解码前处理。连续三次解码失败或持久上游错误会让命令非零退出并保留更新事件；错误样本不保存响应正文、查询密钥或原始解码器详情。

示例：

```bash
litradar index \
  --secret-key-file secrets/litradar.key \
  --file english_journals.csv \
  --update \
  --notify \
  --notify-dry-run
```

## `notify` 和 `push`

两个子命令共享 parser：

```text
litradar notify --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]
    [--db NAME]
    [--state-dir PATH]
    [--changes-file PATH]
    [--ai-model MODEL]
    [--max-candidates N]
    [--timeout N]
    [--retries N]
    [--dedupe-retention-days N]
    [--dry-run | --no-dry-run]

litradar push --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]
    [--db NAME]
    [--state-dir PATH]
    [--changes-file PATH]
    [--ai-model MODEL]
    [--max-candidates N]
    [--timeout N]
    [--retries N]
    [--dedupe-retention-days N]
    [--dry-run | --no-dry-run]
```

parser 还接受 `--index-db PATH` 直接指定索引文件；普通使用优先选择 `--db`。

| 参数                         | 默认值             | 含义                            |
| ---------------------------- | ------------------ | ------------------------------- |
| `--secret-key-file PATH`     | 必填               | 解密用户投递凭据                |
| `--index-db PATH`            | 空                 | 直接指定一个索引 SQLite         |
| `--db NAME`                  | 全部索引库         | 数据库文件名或 stem             |
| `--state-dir PATH`           | 见下表             | 覆盖状态目录                    |
| `--changes-file PATH`        | 自动解析/状态差异  | 指定变更清单                    |
| `--ai-model MODEL`           | 用户设置或代码默认 | 覆盖模型名，不提供 API key      |
| `--max-candidates N`         | `120`              | 进入模型前的候选上限            |
| `--timeout N`                | `60`               | AI/PushPlus HTTP 超时秒数       |
| `--retries N`                | `3`                | CLI 级重试次数，范围 `0..=10`   |
| `--dedupe-retention-days N`  | `60`               | 去重记录保留天数                |
| `--dry-run` / `--no-dry-run` | 执行模式           | 是否禁止外部发送和收藏/去重写入 |

默认状态目录：

| 子命令   | 目录                     |
| -------- | ------------------------ |
| `notify` | `data/push_state`        |
| `push`   | `data/folder_push_state` |

`--db` 省略时按名称排序处理全部 `data/index/*.sqlite`。`utd24` 和 `utd24.sqlite` 等价；路径部分会被去掉。

`--retries 0` 表示只执行首次请求、不再重试；默认值为 3。大于 10 的值会在密钥、数据库、目标和传输初始化前被拒绝。该参数是每个适用传输或 AI 响应格式的重试次数，不是作业总时限或全局请求总数。

## `scheduler`

```text
litradar scheduler validate
    --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]

litradar scheduler run-once TASK_ID
    --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]

litradar scheduler dry-run-once TASK_ID
    --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]
```

| 子命令         | 行为                               |
| -------------- | ---------------------------------- |
| `validate`     | 加载并校验保存的类型化任务，不执行 |
| `run-once`     | 立即执行一个任务                   |
| `dry-run-once` | 立即按 dry-run 模式执行一个任务    |

保存的任务只能展开为同一 `litradar` 可执行文件的 `index`、`notify` 或 `push` argv，不执行 shell 文本。

## `openapi`

```text
litradar openapi [--output PATH]
```

不传 `--output` 时把格式化 JSON 写到 stdout；传入路径时写入该文件。该子命令不需要数据库或部署密钥，也不启动 HTTP/调度运行时。

## 输出和失败

- `serve` 是唯一长驻子命令；正常 SIGINT/SIGTERM 返回 0。
- 维护和作业子命令成功时向 stdout 输出 JSON。
- `openapi` 输出 OpenAPI JSON 或写入指定文件。
- 错误写入 stderr，并以非零状态退出。
- 不支持的位置参数或未知选项会 fail loud，不会静默忽略。
- 密文和密码不会出现在结构化输出。
