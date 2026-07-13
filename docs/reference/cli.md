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

备份命令不接收部署密钥。清单格式名固定为 `litradar-backup`。`--include-push-state` 同时选择 `data/push_state` 和 `data/folder_push_state`。

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
