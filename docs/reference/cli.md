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

`serve` 是唯一常驻入口。它先准备和迁移存储，再在一个进程中并发运行 HTTP 与内嵌调度。计划任务使用当前 `litradar` 可执行文件启动类型化子命令进程，并把每次运行隔离到 Unix process group 或 Windows Job Object。SIGINT/SIGTERM 会先终止完整进程树、等待直接子进程，再保存 `cancelled`；任一运行组件意外失败会关闭另一组件并使进程非零退出。

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

备份命令不接收部署密钥。清单格式名固定为 `litradar-backup`；新备份使用 version 2，并始终包含认证库和完整 `data/meta` 普通文件树。`--include-indexes` 只选择 `data/index` 下受当前二进制支持的精确 v6/v7 内容库，明确排除可重建的 `data/index-control`（项目 `index-batches.sqlite` 和每个 catalog control）以及 `data/index-work`（Crossref 工作集）；`--include-push-state` 同时选择 `data/push_state` 和 `data/folder_push_state`。验证和恢复仍接受 version 1；v1 恢复不会修改目标 Meta 目录。精确替换和离线门禁见[备份与恢复](../operations/backup.md)。

### 索引存储优化

```text
litradar admin index optimize-storage
    --confirm-index-maintenance
    [--project-root PATH]
```

这是显式、离线、整目录替换操作，不接受 `--auth-db` 或部署密钥。它把受支持的精确 v6/v7 内容库从规范化关系表重建为精确 v7，删除 stored-content FTS 的重复内容并压缩 freelist；普通 `serve`、查询和索引启动只 preflight 精确 v6/v7，不会自动升级 v6。

运行前必须停止 `serve`、独立 `index`/投递命令和计划任务子进程，等待 API/worker/调度心跳超过 90 秒，并等待所有 batch/catalog lease 到期或由正常退出释放。先创建并独立验证带 `--include-indexes` 的备份；需要旧二进制降级时，必须保留这份 v6 备份。可用空间至少按 `2 × source_bytes + 64 MiB` 预留；命令也会在复制前记录 `temporary_bytes_required` 估计。

成功 stdout 为一行 JSON：

- 顶层 `status` 为 `optimized` 或空索引目录的 `noop`，`report.outcome` 使用相同 snake-case 值；
- `report` 包含 `database_count`、`source_bytes`、`temporary_bytes_required`、`optimized_bytes`、`reclaimed_bytes` 和 `databases`；
- 每个数据库包含安全文件名、`source_schema_version`、`target_schema_version`、权威 `row_counts`，以及 `before`/`after` 的 `file_bytes`、页大小/页数、freelist、FTS 分配和 `has_content_shadow`。

优化器在任何目录切换前验证源库 `quick_check`、外键、FTS rowid 和固定查询语料；候选还必须通过精确 v7 schema、权威表计数/键集合、无影子表和 freelist 不超过 1%。切换后会再次执行完整验证，失败时自动回滚；成功后才删除 rollback 和维护标记。

失败 stdout 先输出 `{"status":"failed","error":...}`，命令随后非零退出。稳定 `error.code` 包括：

- `confirmation_required`、`active_target`、`active_lease`；
- `interrupted_state`、`invalid_layout`、`unsupported_schema`；
- `validation_failed`、`source_changed`、`io`、`sqlite`；
- `operation_failed`、`rollback_failed`。

只有需要人工恢复的失败才提供非空 `error.recovery_paths`，其中包含精确 `marker`、`staging` 和 `rollback` 路径。不要删除、合并或重命名这些证据；保留完整 JSON 和日志后按路径判断恢复。运维顺序和 Docker 命令见[备份与恢复](../operations/backup.md)与[Docker 部署](../operations/docker.md#离线索引存储优化)。

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
    [--full-rescan | --no-full-rescan]
    [--notify | --no-notify]
    [--notify-dry-run | --no-notify-dry-run]
    [--acknowledge-unknown-notify]
```

| 参数                                       | 默认值   | 含义                                                         |
| ------------------------------------------ | -------- | ------------------------------------------------------------ |
| `--secret-key-file PATH`                   | 必填     | 解密索引运行配置                                             |
| `--file FILE`、`-f FILE`                   | 全部 CSV | 只处理 `data/meta/` 下的一个文件                             |
| `--workers N`、`-w N`                      | `6`      | 每个期刊子进程内的 CNKI 详情请求和 OpenAlex DOI 增强并发上限 |
| `--processes N`                            | `1`      | 单个 CSV 的独立期刊子进程数                                  |
| `--issue-batch N`                          | `8`      | 旧 active batch 的恢复兼容值；当前 Provider 不读取该值       |
| `--timeout N`                              | `20`     | 上游 HTTP 超时秒数                                           |
| `--resume` / `--no-resume`                 | 开启     | 续跑兼容 active batch，或显式放弃它并从 committed anchor 新建 batch |
| `--update` / `--no-update`                 | 关闭     | 是否执行成功期次边界增量并生成变更清单                       |
| `--full-rescan` / `--no-full-rescan`       | 关闭     | 是否扫描完整 Provider 历史且不生成变更清单                   |
| `--notify` / `--no-notify`                 | 关闭     | 更新成功后启动 `litradar notify`                             |
| `--notify-dry-run` / `--no-notify-dry-run` | 关闭     | 下游 notify 是否 dry-run                                     |
| `--acknowledge-unknown-notify`              | 关闭     | 审核 Unknown handoff 后确认并创建新的 notify attempt          |

约束：

- `workers` 与 `processes` 的通用范围均为 `1..=32`，并且 `workers × processes` 不得超过 32。遗留的 `issue-batch` 仍必须至少为 1，以保持既有 batch fingerprint 和 ledger 校验；显式传入会在数据库或 Provider 访问前发出一次固定字段警告。
- 只要选中的目录路由到 Scholarly，`workers` 进一步限制为最多 6、`processes` 最多为 3；超限会在上游请求前失败。国内 CNKI 使用通用 `workers <= 32` 和聚合 32 上限。
- 国内 CNKI 中，`processes` 并行不同期刊，`workers` 是每个期刊子进程在 Provider 构造时创建一次的固定详情线程池；所有 papers 页复用该池，Provider 释放时关闭并等待全部线程。期刊定位、刊期树、papers 页、checkpoint 和 SQLite 提交仍保持有序。实际详情在途量不超过 `workers × min(processes, 期刊数)`、聚合上限 32 和各当前 papers 页的文章数。
- 只要选中的目录路由到 Scholarly，OpenAlex key、Semantic Scholar key 和 Crossref mailto 都必须存在；缺少任一类会在创建内容库、控制库或其他索引状态前失败。
- `--update` 与 `--full-rescan` 互斥；冲突会在数据库迁移、Provider 构造和 worker 启动前失败。
- `--notify` 必须和 `--update` 同时使用。
- 单独传 `--notify-dry-run` 不会启动 notify；它只修改 `--notify` handoff 的模式。
- `--acknowledge-unknown-notify` 必须与默认 `--resume`、`--update` 和 `--notify` 同时使用；它是恢复控制，不进入 batch correctness fingerprint。
- Scholarly 中的 `--workers` 只扩大每个期刊子进程的 OpenAlex DOI 子批在途容量；`6 × 3` 因此最多同时保留 18 个这类请求。每个 OpenAlex key 跨全部期刊子进程共享一组 11-ms 相位，约暴露 `90.9 req/s/key`；增加进程只改变相位所有权，不把单 key 速率乘以进程数。调度器使用全部健康 key，并按剩余 daily credits、在途、冷却和认证状态负载均衡。每日安全预留按 `workers × processes × 最大已知单次 credit cost` 计算。
- Crossref 不使用 `--workers`。整个父进程树共享一个 110-ms polite 相位序列，约 `9.09 req/s`，最多由三个期刊子进程各保留一个在途请求。仅第一个稳定 mailto 被发送；增加 mailto 不会增加 10-RPS/并发-3 合同容量。
- Semantic Scholar 不使用 `--workers`。每个合法 key 各有一个跨进程 1,100-ms 相位序列，约 `0.909 req/s/key`；不同 key 在周期内均匀错开，所以两个或三个 key 可线性增加建模容量。增加 `--processes` 只分配每 key 的相位所有权，不突破 `1 req/s/key`。401/403 只禁用对应 slot，429/Retry-After 只冷却对应 slot，重试同样必须取得未来相位。
- 这些共同 epoch 只协调同一条 `litradar index` 命令的父进程树，不协调其他命令、主机或应用。实际吞吐受 `min(Provider 预算, 在途容量 / 响应延迟, 产生工作速率)` 约束；低 worker、慢响应或工作不足时不会达到理论 RPS。上游临时降额或其他客户端共享 key 时仍可能返回 429，CLI 不承诺精确 100% 利用率或普遍零限流。
- 多个 CSV 仍逐个处理。
- `workers=6`、`processes=1` 是约 100 MiB 索引内存目标下的默认并发。在上述 Provider 约束内显式提高这两个值仍受支持，但可能超过该预算；`issue-batch` 不参与当前运行时并发或内存控制。

索引多进程也通过当前可执行路径启动 `litradar index` 的内部工作请求；不依赖另一个程序名。每个 worker 都在独立的 Unix process group 或 Windows Job Object 中启动，父进程错误、协议失败和清理路径会终止并等待整个进程树。调度父进程同样通过当前二进制启动类型化子命令，并用经过校验的隐藏内部参数关联 `parent_run_id`。手动投递 dispatcher 还会启动私有 `delivery-run --run-id ... --owner-id ...`，child 只从认证 SQLite 和部署密钥加载权威配置。私有命令必须同时携带内部 parent marker，不出现在 `--help`，也不是用户可配置的 CLI。同步公共 CLI 命令不创建 Tokio 工作线程池，只有 `serve` 使用固定为 2 个工作线程的小型异步运行时。

命令结果保持原有顶层 `status`、`message` 和 `csvs` 字段；不含密钥的 `effective_concurrency` 为 JSON 兼容继续保留 `workers`、`processes` 和遗留 `issue_batch`，并明确给出 configured/effective workers、processes、aggregate capacity 以及固定 aggregate limit。只有 workers/processes 字段描述当前并发；`issue_batch` 是恢复兼容元数据。国内 CNKI 每批还记录 `index.provider.concurrency` 结构化事件，其中包含实际创建线程数和本次运行观测到的详情请求峰值。每个 CSV 结果使用定长的 `written_article_count`；旧的 `written_article_ids` 列表不再返回。内部索引工作进程同样只返回计数，避免结果大小随文章数量增长。

发布镜像把 bundle 固定放在 `/usr/share/litradar/meta`。普通 `index` 仅在精确的 `bundle-manifest.json` 存在时，于认证库迁移后、读取密钥和运行设置前准备持久的 `<project-root>/data/meta`，再进入下述规范目录校验；内部多进程 worker 请求不会重复准备。准备结果产生 `storage.managed_meta.prepared` 聚合事件，不改变上述 stdout JSON。该路径不接受环境变量或 CLI 覆盖；本地构建通常发现不到 manifest，因此执行 no-op。运行目录缺失会明确失败，存在但没有选中 CSV 时返回 `skipped`。

### 规范目录和 Provider 路由

显式传入 `--file` 时只接受 `data/meta` 下一个不带目录组件的 `.csv` 文件名；未传入时按文件名顺序处理全部 CSV。每个选中 CSV 只读取一次：同一份字节同时用于摘要、UTF-8/目录校验和本次冻结条目，后续执行不会从路径重读。单文件和全部文件是不同的 batch selection；active all-CSV batch 不能被 `--file` 静默接管。每个文件 stem 稳定决定内容库和控制库：

```text
data/meta/<stem>.csv
data/index/<stem>.sqlite
data/index-control/<stem>.sqlite
```

CSV 使用 LitRadar 维护的 `catalog_id,title,issn,eissn,all_issns,title_aliases,area,...rankings` 契约，没有 `source` 或上游 ID。解析器在网络请求前拒绝未知列、非法/重复 `catalog_id`、非法 ISSN、重复别名和不规范文本。

`index_provider_routes` 从 `auth.sqlite.runtime_settings` 把 stem 映射到一个已注册 `IndexContentProvider`。缺少 route、Provider 未注册或没有索引 capability 都会在启动 worker 前失败。改变 route 不改目录或内容库身份；在线摘要页和全文使用各自的 default + per-catalog 顺序，和索引 Provider 单选相互独立。

内容库必须是新建/空 v0、精确 v6/v7，或可事务迁移到 v7 的精确 v4/v5。新文件使用 v7；普通启动对精确 v6/v7 只做 preflight，不自动把 v6 改为 v7。非空 v0 及 v1–v3 会返回包含确切路径的 rebuild-required 错误；命令不自动删除、改名或降低 `user_version`。先备份，再移动或删除点名文件并重建。

### 实时恢复与增量同步

每条命令先在 `data/index-control/index-batches.sqlite` 取得项目级 lease，再为当前目录/Provider 在 `data/index-control/<stem>.sqlite` 取得独立 lease。父进程每 30 秒续期到未来 300 秒；未过期所有者会在调用上游前阻止新的竞争命令。正常结束释放 lease；进程被强制终止时，先确认旧进程已经消失并等待 lease 过期，不要同时启动第二个索引进程。

默认 `--resume` 的边界是“兼容的 active project batch”，不是所有历史成功状态。batch 指纹覆盖：

- `--file` 或全部 CSV 的选择方式、按文件名排序后的 catalog 顺序，以及每个 CSV 的精确字节；
- 每个 stem 的 `index_provider_routes` 结果；
- Bootstrap / Incremental / FullRescan 模式、遗留 `--issue-batch` 恢复值、notify 和 notify dry-run 选择。

`issue-batch` 只因旧 ledger 的恢复兼容而继续进入 correctness fingerprint；当前 Provider 不使用它决定请求、分页、并发、吞吐或内存。显式传入该参数会产生一次不含数值、路径、凭据或 cursor 的结构化警告。`workers`、`processes`、timeout、代理和凭据不影响 correctness fingerprint。兼容 active batch 会按持久顺序跳过已经 completed 的 catalog 和同 batch 已完成的 journal，从第一个未完成 traversal checkpoint 继续；CSV、顺序、selection、route、模式或上述正确性选项变化会在 Provider 访问前 fail closed，并只报告差异类别。一个 batch 全部成功后进入 completed；下一次命令总会创建新 batch 并重新检查全部选中 journal，旧成功行只作为增量 anchor，不是永久 skip 标记。

`--no-resume` 明确放弃当前 active batch，并在清理该 batch 自有的 `provider_run_checkpoints` 后创建新 batch。它保留 committed anchors、内容库、outbox 和已经发布的 manifest；新 traversal 从所选模式和现有 committed anchor 开始。若 ledger 已记录一个待完成的已发布 notify manifest，命令会拒绝放弃，必须先用原 correctness inputs 恢复，并在必要时显式确认 Unknown；因此 `--no-resume` 不能静默跳过该 handoff。它不是“忽略一个 CSV 错误继续”，也不会合并不兼容的冻结输入。

控制库把成功状态与运行状态分开保存，并以目录、Provider、`catalog_id` 隔离。命令模式如下：

- 不传 `--update` 或 `--full-rescan` 时使用 Bootstrap。同 active batch 已完成的 journal 可零请求跳过；新 batch 会重新执行完整覆盖，即使旧 anchor 为 NULL。
- `--update` 使用 Incremental。从远端当前头部扫描到上一次完整成功 anchor，并完整包含该边界期次；没有成功行或成功 anchor 为 NULL 时安全执行完整覆盖。只有该模式在成功后发布 `.changes.json`。同 active batch 已完成的 journal 才跳过。
- `--full-rescan` 使用 FullRescan，忽略 committed anchor 作为停止边界并核对完整 Provider 历史。它可以恢复同 batch、同模式和同 base 的 traversal checkpoint；同 batch 已完成的 journal 可跳过。该模式不发布 `.changes.json`，因此不能与 `--notify` 组合。

一次 journal 运行开始时冻结 `base_anchor`；Provider 在确认候选顺序后冻结自己的 candidate head，Crossref 要先收齐并验证创建日期分片，再按本地期次组选择。恢复只接受 batch ID、同步模式和 base 都匹配的运行，模式或 batch 不一致会 fail closed。batch ID 只属于核心控制状态，不进入 Provider context 或 worker request JSON。

每页先在内容库事务中写入规范 journal/issue/article、identity aliases、投影和 change outbox，再推进 traversal checkpoint。最终内容批次提交后，核心才在一个控制事务中删除运行 checkpoint 并替换 committed anchor。内容成功而控制提交失败时，旧 anchor 不变；重跑冻结窗口并依靠 alias/upsert 去重。

切换 Provider 会使用没有 anchor 的新 namespace；删除控制库也会同时失去成功 anchor 和运行进度。两种情况都安全退回完整覆盖，不触碰内容库，也不会复制文章或改变 ID。

`--update` 从内容库的事务性 `article_change_events` 生成 Provider-neutral changes JSON。核心先把精确 payload、目标相对路径和 through-event cursor 持久化为 batch manifest intent，再原子发布相同字节、幂等清理该 cursor，最后进入可选 notify phase。重启可只补 manifest 或 notify，不重复已完成 Provider 工作。若 outbox 已空但已有一个有界且可解析、属于同内容库的 manifest，空 update 会保留该文件且不再次 notify。batch ledger 丢失时文件/SQLite 边界仍按至少一次处理，消费者必须按规范文章身份去重。Provider 请求统计只在终态结构化日志中聚合，不写入内容库。

notify phase 使用 disposable batch schema v2 的独立 typed handoff state，不再把 exit code 混入不可变 catalog outcome。父进程在启动 child 前持久化 32 位十六进制 attempt ID；child 只向父进程输出一行 compact JSON（protocol、attempt、workflow、mode、status、db），父进程最多保留 64 KiB stdout、继续排空管道，并严格核对上下文和退出类别。catalog 只有在 `idle`、`completed` 或 `skipped` 且退出 0 时才能完成。

恢复规则按状态固定：结果尚未落盘或 child 返回 `running` 时复用同一 attempt ID，以便投递层返回已有 durable run；`failed`、`cancelled` 或 `timed_out` 在下一次 operator invocation 使用新 attempt；`unknown`、缺失/畸形/超大输出、上下文不匹配或 status/exit 不一致都不会自动启动 child。审核 delivery run/item/dedupe 后，可用原命令加 `--acknowledge-unknown-notify`；确认 ID 与时间会和新 attempt 在一个 transaction 中写入。attempt 只改变外层 scheduled run 身份，文章级 Confirmed/Unknown dedupe 跨 attempt 保留，所以已确认或不确定的文章不重发，新文章仍可投递。每次父进程调用对每个 catalog 最多启动一个 notify child。

### Crossref 2026-08 游标升级后的 English 恢复

新版按 [Crossref 官方建议](sources/scholarly.md#crossref-分页)使用完整 created 历史分片、固定 update 条件和计数校验；无序结果验证后才本地排序并输出。没有新增公共 CLI 参数。临时工作集固定在 `data/index-work/scholarly/`，不参与备份，不放在 `/tmp`；响应上限仍是 16 MiB，单工作集主文件最多 4 GiB，事务日志另占磁盘。

确认旧索引进程已停止、没有未过期的竞争 lease 后，在项目根目录使用新二进制恢复原来关闭通知的 English 增量批次：

```bash
litradar index \
  --secret-key-file secrets/litradar.key \
  --project-root . \
  --file english_journals.csv \
  --update \
  --resume \
  --no-notify
```

保留原 CSV、模式和 batch 正确性选项，不删除内容库、控制库或 batch ledger，也不加 `--no-resume`。旧 Scholarly v1 Crossref traversal 会保留 base/candidate 并从新查询头安全重放；v1 OpenAlex traversal 保留原 cursor 语义；NULL traversal 正常开始。新 traversal 为 v2，成功 anchor 仍为 v1，正式 schema 不变。v2 写入后不能盲目用旧二进制接管。

同一次 resume 可以继续已验证片和有效 cursor，不再按 240 秒或 HTTP 500 重扫。下一次独立 update 仍会重新查询并补查整个成功边界期次，日期下界为 anchor 年份的 1 月 1 日；没有固定回看 7/30 天的逻辑，终点 cursor 也不是永久水位。缺失工作集时按原 `C/T`、update 条件和 candidate 重建，不能在丢失前半结果时继续旧 cursor。

只有命令成功结束、同 batch 的全部选中 journal/catalog 完成、变更清单可解析且没有残留 traversal 或活动 lease，才确认本次更新完成。失败时保留日志、正式内容和恢复状态；不要用删除数据库来处理计数或容量错误。交互观察应预先限定，例如最多 3 分钟、3 次状态检查；到达界限只报告实际进度并停止观察，不把启动或部分处理算作完成，也不因此终止健康的索引进程。固定 cursor、时间上界和计数相等不提供远端快照保证。

### 从 control v3 恢复旧 English traversal

从 control v3 升级留下的 batchless traversal 只允许由显式单 CSV、默认 `--resume` 接管；隐式全部 CSV 会拒绝，以免把不同旧 epoch 混入一个 batch。对已有 English 失败状态，先确认没有旧索引进程，再沿用原命令的 mode、ledger 中保存的遗留 issue-batch 恢复值和 notify 选项，并增加下列参数。只有原值不是默认 `8` 时才需要显式传入 `--issue-batch`；此时预期会看到兼容性警告：

```bash
litradar index \
  --secret-key-file /run/secrets/litradar.key \
  --project-root /app \
  --file english_journals.csv \
  --update
```

不要为这次 legacy 接管添加 `--no-resume`。核心要求旧 batchless checkpoints 共享一个 mode 和 start epoch，把同 epoch 已完成 anchor 绑定到新 batch，然后跳过较早 journal 并从保留的 English checkpoint（例如 Public Choice）继续。若此前已经用新版本启动过不带 `--file` 的失败尝试，active all-CSV batch 会产生 `catalog_selection` mismatch；确认进程已停止后，先移动或删除仅项目级的 `data/index-control/index-batches.sqlite`，保留 `english_journals.sqlite`，再执行上述显式恢复命令。

日常增量及可选通知：

```bash
litradar index \
  --secret-key-file secrets/litradar.key \
  --file english_journals.csv \
  --update \
  --notify \
  --notify-dry-run
```

周期性核对历史回填和旧元数据（与 `--update` 互斥，不生成 changes JSON）：

```bash
litradar index \
  --secret-key-file secrets/litradar.key \
  --file english_journals.csv \
  --full-rescan
```

## `notify` 和 `push`

两个子命令共享 parser：

```text
litradar notify --secret-key-file PATH
    [--project-root PATH]
    [--auth-db PATH]
    [--db NAME]
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
| `--changes-file PATH`        | SQLite checkpoint 差异 | 指定 Provider-neutral 变更清单 |
| `--ai-model MODEL`           | 用户设置或代码默认 | 覆盖模型名，不提供 API key      |
| `--max-candidates N`         | `120`              | 进入模型前的候选上限            |
| `--timeout N`                | `60`               | AI/PushPlus HTTP 超时秒数       |
| `--retries N`                | `3`                | 适用请求的重试次数，范围 `0..=10` |
| `--dedupe-retention-days N`  | `60`               | 已确认去重记录保留天数          |
| `--dry-run` / `--no-dry-run` | 执行模式           | 是否禁止外部发送和收藏/去重写入 |

checkpoint、run、item、dedupe 和 workflow lease 统一写入 `--auth-db` 指向的认证 SQLite，不再接受状态目录覆盖。启动时会安全导入项目根下保留的旧 `<db>.json`，但运行过程中只读取 `.changes.json`，不会创建或更新投递状态 JSON。

`--db` 省略时按名称排序处理全部 `data/index/*.sqlite`。`utd24` 和 `utd24.sqlite` 等价；路径部分会被去掉。

`--retries 0` 表示只执行首次请求、不再重试；默认值为 3。大于 10 的值会在密钥、数据库、目标和传输初始化前被拒绝。该参数是每个适用请求或 AI 响应格式的重试次数，不是作业总时限或全局请求总数。AI 可对连接失败、timeout 和受限瞬态状态重试；PushPlus 仅在连接建立明确失败、请求尚未发送时重试。一旦 PushPlus 请求可能到达上游，timeout、HTTP 响应或连接后错误会直接产生 `unknown`，不会自动重放。`--dedupe-retention-days <= 0` 禁用确认记录清理，而不是立即删除全部记录。

`notify`/`push` 在投递运行已形成聚合结果时总会先向 stdout 输出一行完整 JSON。聚合状态为 `completed`、`skipped` 或 `idle` 时退出 0；`running`、`cancelled`、`timed_out`、`failed` 或 `unknown` 时退出非零。这样普通调用方仍能解析每个数据库和订阅者的精确结果，scheduler 不会把业务失败误记为成功。`index --notify` 使用上述私有 compact handoff 契约并同时核对 typed status 与退出类别；隐藏参数不属于公开 CLI，也不会出现在 help 中。

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
