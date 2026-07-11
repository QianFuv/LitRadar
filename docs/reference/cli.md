# CLI 参考

本文档是 Rust 后端命令、参数和默认值的唯一完整参考。任务流程分别见[开发指南](../guides/development.md)、[Docker 部署](../operations/docker.md)、[通知与追踪](../guides/notifications.md)和[备份与恢复](../operations/backup.md)。

## 调用形式

本地开发：

```bash
cargo run --bin <command> -- <arguments>
```

已安装二进制或容器内：

```bash
<command> <arguments>
```

使用 `--help` 或 `-h` 查看当前命令接受的概要语法。

## 公共路径参数

除 `api` 外，Rust CLI 共享：

| 参数                  | 默认值                            | 含义                                     |
| --------------------- | --------------------------------- | ---------------------------------------- |
| `--project-root PATH` | 当前工作目录                      | 解析 `data/`、`libs/` 和相对路径的根目录 |
| `--auth-db PATH`      | `<project-root>/data/auth.sqlite` | 显式认证/业务数据库                      |

`api` 接受 `--project-root`，但不接受 `--auth-db`；它始终使用项目根下的 `data/auth.sqlite`。

相对路径按 `project-root` 解析，绝对路径保持不变。

## `api`

```text
api --secret-key-file PATH
    [--host HOST]
    [--port PORT]
    [--project-root PATH]
    [--require-secure-cookies]
```

| 参数                       | 默认值       | 含义                                                 |
| -------------------------- | ------------ | ---------------------------------------------------- |
| `--secret-key-file PATH`   | 必填         | 32 字节部署密钥                                      |
| `--host HOST`              | `127.0.0.1`  | 监听地址                                             |
| `--port PORT`              | `8000`       | TCP 端口                                             |
| `--project-root PATH`      | 当前工作目录 | 数据和扩展根目录                                     |
| `--require-secure-cookies` | 关闭         | 要求数据库 `secure_cookies=true`，否则绑定端口前失败 |

`api` 是规范入口。workspace 和后端镜像还包含行为相同的包名二进制 `litradar-api`；文档和部署命令统一使用 `api`。

## `admin`

`admin` 是本机维护入口，不启动网络服务。

### 初始化管理员

```text
admin bootstrap
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
admin secrets migrate
    --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]

admin secrets verify
    --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]
```

`migrate` 把明文秘密转换为 `litradarenc:v1:`；`verify` 只验证当前密文。操作顺序见[安全说明](../operations/security.md)。

### 轮换部署密钥

```text
admin secrets rotate
    --old-key-file PATH
    --new-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]
```

两个 key 文件都必须是 32 个原始字节。

### 备份

```text
admin backup create
    --output PATH
    [--include-indexes]
    [--include-push-state]
    [--project-root PATH]
    [--auth-db PATH]

admin backup verify
    --backup PATH
    [--project-root PATH]

admin backup restore
    --backup PATH
    --confirm-restore
    [--project-root PATH]
    [--auth-db PATH]
```

备份命令不接收部署密钥。清单格式名固定为 `litradar-backup`，不接受改名前的格式。`--include-push-state` 同时选择 `data/push_state` 和 `data/folder_push_state`。

## `index`

```text
index --secret-key-file PATH
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

| 参数                                       | 默认值    | 含义                               |
| ------------------------------------------ | --------- | ---------------------------------- |
| `--secret-key-file PATH`                   | 必填      | 解密 scholarly 运行配置            |
| `--file FILE`、`-f FILE`                   | 全部 CSV  | 只处理 `data/meta/` 下的一个文件   |
| `--workers N`、`-w N`                      | `32`      | 每个期刊进程内的 CNKI 文章详情并发 |
| `--processes N`                            | `2`       | 单个 CSV 的期刊进程数              |
| `--issue-batch N`                          | `workers` | 每轮合并的 CNKI issue 数           |
| `--timeout N`                              | `20`      | 上游 HTTP 超时秒数                 |
| `--resume` / `--no-resume`                 | 开启      | 是否跳过已完成期刊/年份            |
| `--update` / `--no-update`                 | 关闭      | 是否生成增量变更清单               |
| `--notify` / `--no-notify`                 | 关闭      | 更新成功后启动 `notify`            |
| `--notify-dry-run` / `--no-notify-dry-run` | 关闭      | 下游 notify 是否 dry-run           |

约束：

- `workers`、`processes`、`issue-batch` 必须至少为 1。
- `--notify` 必须和 `--update` 同时使用。
- 单独传 `--notify-dry-run` 不会启动 notify；它只修改 `--notify` handoff 的模式。
- `--workers` 不扩大 scholarly 请求并发；Semantic Scholar 按 `processes` 做进程感知错峰。
- 多个 CSV 仍逐个处理。

示例：

```bash
index \
  --secret-key-file secrets/litradar.key \
  --file english_journals.csv \
  --update \
  --notify \
  --notify-dry-run
```

## `notify` 和 `push`

两个命令共享 parser：

```text
notify|push --secret-key-file PATH
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

当前 parser 还接受 `--index-db PATH` 直接指定索引文件，但帮助字符串尚未列出该参数。普通使用优先选择 `--db`。

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

| 命令     | 目录                     |
| -------- | ------------------------ |
| `notify` | `data/push_state`        |
| `push`   | `data/folder_push_state` |

`--db` 省略时按名称排序处理全部 `data/index/*.sqlite`。`utd24` 和 `utd24.sqlite` 等价；路径部分会被去掉。

`--retries 0` 表示只执行首次请求、不再重试；默认值为 3。大于 10 的值会在密钥、数据库、目标和传输初始化前被拒绝。该参数是每个适用传输或 AI 响应格式的重试次数，不是作业总时限或全局请求总数。

## `scheduler`

```text
scheduler validate
    --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]

scheduler run-once TASK_ID
    --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]

scheduler dry-run-once TASK_ID
    --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]
```

| 子命令         | 行为                               |
| -------------- | ---------------------------------- |
| `validate`     | 加载并校验保存的类型化任务，不执行 |
| `run-once`     | 立即执行一个任务                   |
| `dry-run-once` | 立即按 dry-run 模式执行一个任务    |

旧 `legacy_command` 任务保持禁用，不能被 `run-once` 执行。

## `worker`

```text
worker --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]
    [--interval-seconds N]
    [--max-iterations N]
```

| 参数                     | 默认值 | 含义                             |
| ------------------------ | ------ | -------------------------------- |
| `--secret-key-file PATH` | 必填   | 解密任务需要的凭据               |
| `--interval-seconds N`   | `30`   | 调度轮询间隔                     |
| `--max-iterations N`     | 无限   | 有限循环，主要用于测试和受控运行 |

worker 迁移数据库后进入循环，按持久化游标回看最多 24 小时的 UTC 分钟槽。

## 输出和失败

- 维护和作业命令成功时向 stdout 输出 JSON。
- 错误写入 stderr，并以非零状态退出。
- `api` 和 `worker` 是长驻进程。
- 不支持的位置参数或未知选项会 fail loud，不会静默忽略。
- 密文和密码不会出现在结构化输出。
