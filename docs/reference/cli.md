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

备份命令不接收部署密钥。清单格式名固定为 `litradar-backup`；新备份使用 version 2，并始终包含认证库和完整 `data/meta` 普通文件树。`--include-indexes` 只选择 `data/index` 下的 v4 内容库，明确排除可重建的 `data/index-control`；`--include-push-state` 同时选择 `data/push_state` 和 `data/folder_push_state`。验证和恢复仍接受 version 1；v1 恢复不会修改目标 Meta 目录。精确替换和离线门禁见[备份与恢复](../operations/backup.md)。

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

| 参数                                       | 默认值   | 含义                                                         |
| ------------------------------------------ | -------- | ------------------------------------------------------------ |
| `--secret-key-file PATH`                   | 必填     | 解密索引运行配置                                             |
| `--file FILE`、`-f FILE`                   | 全部 CSV | 只处理 `data/meta/` 下的一个文件                             |
| `--workers N`、`-w N`                      | `6`      | 每个期刊子进程内的 CNKI 文章详情和 OpenAlex DOI 增强并发上限 |
| `--processes N`                            | `1`      | 单个 CSV 的独立期刊子进程数                                  |
| `--issue-batch N`                          | `8`      | 每轮合并的 CNKI issue 数                                     |
| `--timeout N`                              | `20`     | 上游 HTTP 超时秒数                                           |
| `--resume` / `--no-resume`                 | 开启     | 是否跳过已完成期刊/年份                                      |
| `--update` / `--no-update`                 | 关闭     | 是否生成增量变更清单                                         |
| `--notify` / `--no-notify`                 | 关闭     | 更新成功后启动 `litradar notify`                             |
| `--notify-dry-run` / `--no-notify-dry-run` | 关闭     | 下游 notify 是否 dry-run                                     |

约束：

- `workers`、`processes`、`issue-batch` 必须至少为 1。
- 只要选中的目录路由到 Scholarly，`workers` 最多为 6，`processes` 最多为 3；超限会在上游请求前失败。CNKI 路由不受这两个 Scholarly 上限约束。
- 只要选中的目录路由到 Scholarly，OpenAlex key、Semantic Scholar key 和 Crossref mailto 都必须存在；缺少任一类会在创建内容库、控制库或其他索引状态前失败。
- `--notify` 必须和 `--update` 同时使用。
- 单独传 `--notify-dry-run` 不会启动 notify；它只修改 `--notify` handoff 的模式。
- Scholarly 中的 `--workers` 只扩大每个期刊子进程的 OpenAlex DOI 子批在途容量；`6 × 3` 因此最多同时保留 18 个这类请求。每个 OpenAlex key 跨全部期刊子进程共享一组 11-ms 相位，约暴露 `90.9 req/s/key`；增加进程只改变相位所有权，不把单 key 速率乘以进程数。调度器使用全部健康 key，并按剩余 daily credits、在途、冷却和认证状态负载均衡。每日安全预留按 `workers × processes × 最大已知单次 credit cost` 计算。
- Crossref 不使用 `--workers`。整个父进程树共享一个 110-ms polite 相位序列，约 `9.09 req/s`，最多由三个期刊子进程各保留一个在途请求。仅第一个稳定 mailto 被发送；增加 mailto 不会增加 10-RPS/并发-3 合同容量。
- Semantic Scholar 不使用 `--workers`。每个合法 key 各有一个跨进程 1,100-ms 相位序列，约 `0.909 req/s/key`；不同 key 在周期内均匀错开，所以两个或三个 key 可线性增加建模容量。增加 `--processes` 只分配每 key 的相位所有权，不突破 `1 req/s/key`。401/403 只禁用对应 slot，429/Retry-After 只冷却对应 slot，重试同样必须取得未来相位。
- 这些共同 epoch 只协调同一条 `litradar index` 命令的父进程树，不协调其他命令、主机或应用。实际吞吐受 `min(Provider 预算, 在途容量 / 响应延迟, 产生工作速率)` 约束；低 worker、慢响应或工作不足时不会达到理论 RPS。上游临时降额或其他客户端共享 key 时仍可能返回 429，CLI 不承诺精确 100% 利用率或普遍零限流。
- 多个 CSV 仍逐个处理。
- `6/1/8` 是约 100 MiB 索引内存目标下的默认并发。在上述 Provider 约束内显式提高并发仍受支持，但可能超过该预算。

索引多进程也通过当前可执行路径启动 `litradar index` 的内部工作请求；不依赖另一个程序名。同步 CLI 命令不创建 Tokio 工作线程池，只有 `serve` 使用固定为 2 个工作线程的小型异步运行时。

命令结果保持原有顶层 `status`、`message` 和 `csvs` 字段，并新增不含密钥的 `effective_concurrency`，记录本次实际使用的 `workers`、`processes` 和 `issue_batch`。每个 CSV 结果使用定长的 `written_article_count`；旧的 `written_article_ids` 列表不再返回。内部索引工作进程同样只返回计数，避免结果大小随文章数量增长。

发布镜像设置 `LITRADAR_BUNDLED_META_DIR=/usr/share/litradar/meta`。普通 `index` 在认证库迁移后、读取密钥和运行设置前准备持久的 `<project-root>/data/meta`，再进入下述规范目录校验；内部多进程 worker 请求不会重复准备。准备结果产生 `storage.managed_meta.prepared` 聚合事件，不改变上述 stdout JSON。未设置该变量的本地运行不执行受管准备；目录缺失会明确失败，存在但没有选中 CSV 时返回 `skipped`。

### 规范目录和 Provider 路由

显式传入 `--file` 时只接受 `data/meta` 下一个不带目录组件的 `.csv` 文件名；未传入时按文件名顺序处理全部 CSV。每个文件 stem 稳定决定内容库和控制库：

```text
data/meta/<stem>.csv
data/index/<stem>.sqlite
data/index-control/<stem>.sqlite
```

CSV 使用 LitRadar 维护的 `catalog_id,title,issn,eissn,all_issns,title_aliases,area,...rankings` 契约，没有 `source` 或上游 ID。解析器在网络请求前拒绝未知列、非法/重复 `catalog_id`、非法 ISSN、重复别名和不规范文本。

`index_provider_routes` 从 `auth.sqlite.runtime_settings` 把 stem 映射到一个已注册 `IndexContentProvider`。缺少 route、Provider 未注册或没有索引 capability 都会在启动 worker 前失败。改变 route 不改目录或内容库身份；在线详情、摘要和全文顺序另行配置。

内容库必须是新建/空 v0 或精确 v4。非空 v0 及 v1–v3 会返回包含确切路径的 rebuild-required 错误；命令不自动删除、改名、迁移或降低 `user_version`。先备份，再移动或删除点名文件并重建。

### 实时恢复与增量同步

每个目录/Provider 在 `data/index-control/<stem>.sqlite` 取得独立 lease。父进程每 30 秒续期到未来 300 秒；未过期所有者会在调用上游前阻止同一 namespace 的新命令。正常结束释放 lease；进程被强制终止时，确认旧进程已经消失并等待 lease 过期，或在维护窗口删除整个可丢弃控制库后重跑。

Provider checkpoint 以目录、Provider 和 journal/year/listing scope 隔离。普通 `--resume` 可跳过已完成 journal 或从 opaque checkpoint 继续；`--update` 忽略完成 checkpoint 并重新扫描规范内容。切换 Provider 会使用新的 checkpoint namespace，不触碰内容库。

每页先在内容库事务中写入规范 journal/issue/article、identity aliases、投影和 change outbox，再推进控制 checkpoint。内容成功而 checkpoint 失败时，重跑依靠 alias/upsert 去重。删除控制库会失去进度但不会复制文章或改变 ID。

`--update` 从内容库的事务性 `article_change_events` 生成 Provider-neutral changes JSON。worker、上游或清单失败会保留 outbox；文件发布和 SQLite 清理之间是至少一次边界，消费者必须按规范文章身份去重。Provider 请求统计只在终态结构化日志中聚合，不写入内容库。

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
