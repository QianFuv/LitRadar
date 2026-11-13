# 性能剖析

本文档说明服务端容器的性能剖析脚本：Go 镜像画像、Docker 内存画像与门禁，以及日志开销对照。这些脚本用于开发和发布前测量，不是部署步骤；部署见 [Docker 部署](../operations/docker.md)，日志契约见[日志运维](../operations/logging.md)。

所有脚本都会启动迁移并读写传入的数据目录。确定性测量使用隔离副本；只有在停机并完成已验证备份（见[备份与恢复](../operations/backup.md)）后，才可把真实 `./data` 交给更新场景。不要把正在运行的服务所用的数据目录交给画像脚本。

## Go 镜像画像

先构建镜像，再运行脚本：

```bash
docker buildx build -f apps/server/Dockerfile --platform linux/amd64 --load --provenance=false -t litradar:go-test-amd64 .
node apps/server/tests/profiling/go-image.mjs
```

脚本要求原生 amd64 主机，通过 `apps/server/tests/container-smoke.mjs --profile` 执行三轮隔离工作负载，使用 160 MiB 容器上限和 64 MiB tmpfs，验证合成文章的认证检索、真实辅助程序与退出清理，分别报告应用 RSS 和整个 cgroup 的当前/峰值用量。两者记账方式不同，不能相加；脚本也不单独测量辅助程序 RSS。结果保存在 `output/profiling/`。

## Docker 内存画像

PowerShell 7 脚本 `apps/server/tests/profiling/profile_docker_memory.ps1` 为每次运行生成唯一的 Compose 项目、容器和网络，只删除这些具名测试资源，并把不含命令参数和密钥值的 JSON 写入已忽略的 `output/memory/`。Docker 必须使用 cgroup v2，目标镜像必须先构建：

```powershell
docker buildx build -f apps/server/Dockerfile --platform linux/amd64 --load -t ghcr.io/qianfuv/litradar-server:latest .
docker compose config --quiet
```

`-Scenario`（`warm-idle`、`index`、`update`、`scheduled-child`）和 `-DataPath` 是必填参数。脚本把 `-DataPath` 以读写方式挂载到 `/app/data`：服务启动会迁移其中的数据库，索引/更新会写入其中的数据。部署密钥仍由 Compose secret 挂载，`-Command` 只传容器内密钥路径。

### 指标口径

| JSON 字段/来源                   | 含义                                                                         | 用途                               |
| -------------------------------- | ---------------------------------------------------------------------------- | ---------------------------------- |
| `Memory.WorkingSet*`             | `memory.current - memory.stat.inactive_file`，与 Docker working-set 口径一致 | 报告用量；仅执行显式启用的预算     |
| `Memory.CgroupCurrent*`          | 原始 cgroup 当前用量，包含可回收文件页缓存                                   | 分析页缓存和 cgroup 总占用         |
| `Memory.CgroupLifetimePeakBytes` | 容器创建以来的原始 `memory.peak`                                             | 诊断启动或作业瞬时峰值             |
| `PeakProcesses`                  | `docker top` 的进程 RSS、线程数和命令名峰值拆分                              | 区分 `serve`、作业子进程和辅助进程 |
| `Memory.SwapPeakBytes`           | `memory.swap.current` 的采样峰值                                             | 仅启用预算时要求为零               |
| `EventDelta`                     | 新建 cgroup 生命周期内的 `memory.events` 计数                                | OOM 始终失败；启用预算时另检查 max |
| `FullPressureAvg10Max`           | `memory.pressure` 的 full `avg10` 采样最大值                                 | 仅启用预算时要求为零               |

每个样本还保存选定的 `memory.stat`、PSI、进程 RSS 总和、进程数和线程数。摘要报告 working-set 的 p50、p95、采样峰值、持续时间、场景退出码、OOM 状态和门禁失败原因。采样需要短生命周期的 `docker exec`，因此 cgroup 数值略偏保守；进程 RSS 与 cgroup working set 的记账方式不同，不能相加。

### 门禁

默认只报告内存用量：实际 OOM、命令非零退出（含超时 124）、上游错误或测试流量失败使画像失败，通过条件为 JSON 中 `Gate.Passed=true`。显式传入 `-EnforceMemoryBudgets` 时启用以下预算，并要求 swap、`memory.events.max` 和 full PSI `avg10` 均为零：

| 场景                                 | p95     | 采样峰值 |
| ------------------------------------ | ------- | -------- |
| `warm-idle`                          | 20 MiB  | 24 MiB   |
| `index`、`update`、`scheduled-child` | 100 MiB | 120 MiB  |

单独传入 `-P95LimitMiB` 或 `-PeakLimitMiB` 只启用对应阈值；与 `-EnforceMemoryBudgets` 同时使用时，显式值覆盖表中预算。`-ExpectedMemoryLimitMiB` 只核对已有容器上限，不创建上限；`0` 表示期望容器没有上限。未启用的阈值在报告中为 `null`。超限返回 1。调整并发时应比较相同数据下的测量结果；遗留 `--issue-batch` 不控制当前内存或并发。

### 场景命令

真实数据只在停机备份验证完成后画像。建议顺序为默认恢复索引、CCF 更新、中国期刊更新、英文学术更新、同 cgroup 计划任务子进程，最后是预热后的日常服务。`-DurationSeconds` 是保护性超时；作业提前完成时立即结束，超时则以退出码 124 失败。

默认恢复索引不带 `--update`，用于证明已完成期刊可跳过且旧待发布事件不会被非更新运行接管：

```powershell
pwsh ./apps/server/tests/profiling/profile_docker_memory.ps1 `
  -Scenario index `
  -DataPath ./data `
  -DurationSeconds 14400 `
  -Command @(
    'index',
    '--secret-key-file', '/run/secrets/litradar_key'
  ) `
  -OutputPath ./output/memory/final-resume-index.json
```

更新场景依次替换 `--file` 为 `ccf_computer_journals.csv`、`chinese_journals.csv` 和 `english_journals.csv`，并相应修改输出文件名：

```powershell
pwsh ./apps/server/tests/profiling/profile_docker_memory.ps1 `
  -Scenario update `
  -DataPath ./data `
  -DurationSeconds 14400 `
  -Command @(
    'index',
    '--secret-key-file', '/run/secrets/litradar_key',
    '--file', 'ccf_computer_journals.csv',
    '--update'
  ) `
  -OutputPath ./output/memory/final-update-ccf.json
```

常驻服务和同 cgroup 子任务的合并画像：

```powershell
pwsh ./apps/server/tests/profiling/profile_docker_memory.ps1 `
  -Scenario scheduled-child `
  -DataPath ./data `
  -DurationSeconds 14400 `
  -Command @(
    'index',
    '--secret-key-file', '/run/secrets/litradar_key',
    '--file', 'ccf_computer_journals.csv',
    '--update'
  ) `
  -OutputPath ./output/memory/final-scheduled-child.json
```

五分钟预热后采集十分钟日常服务和轻流量；直接画像服务端时路径包含 `/health/live`、`/health/ready` 和 `/docs`：

```powershell
pwsh ./apps/server/tests/profiling/profile_docker_memory.ps1 `
  -Scenario warm-idle `
  -DataPath ./data `
  -WarmupSeconds 300 `
  -DurationSeconds 600 `
  -TrafficPath /health/live,/health/ready,/docs `
  -OutputPath ./output/memory/final-warm-idle.json
```

每份 JSON 只记录 `CommandProvided`，不记录命令参数或秘密。

### 验证门禁本身

以下极小阈值用于确认单项内存检查确实能报错：

```powershell
pwsh ./apps/server/tests/profiling/profile_docker_memory.ps1 `
  -Scenario warm-idle `
  -DataPath ./isolated-profile-data `
  -DurationSeconds 10 `
  -P95LimitMiB 0.001 `
  -PeakLimitMiB 0.001
```

该命令预期返回 1，并在 JSON 的 `Gate.Failures` 中同时列出 p95 和峰值超限。中断或失败时，`finally` 仍只按本次唯一名称删除容器和网络；若宿主机或 Docker daemon 被强制终止，可用 `docker ps -a --filter name=litradar-memory-` 检查后按完整名称清理。

## 日志开销对照

日志或请求路径变更后，先构建服务端镜像，再对隔离数据副本运行三轮 off/on 对照：

```powershell
docker compose build server
pwsh ./apps/server/tests/profiling/profile_logging.ps1 `
  -DataPath ./output/logging-fixture `
  -Rounds 3 `
  -RequestCount 300 `
  -Concurrency 4
```

脚本先用正常容器启动完成迁移，通过可配置的 `sqlite3` CLI（`-SqliteExecutable`）快照 `log_format`/`log_filter`，再事务性切换 off/default 两种模式，并在结束时恢复原始行。它交错运行两种模式，对 `/api/logging-profile-missing` 发起固定 404 请求，验证每个默认模式应用行都是 JSON 且含必填字段，并检查请求事件数与丢失数。报告写入已忽略的 `output/logging/`。脚本还会为两种模式调用 warm-idle 内存画像；默认只报告内存用量，不设置预算，也不假定容器已有内存上限。

门禁：

- 预期负载 `dropped_count=0`
- logging-on p95 延迟增量不超过 `max(2 ms, logging-off p95 × 15%)`
- 两次内存画像都必须通过，包括 OOM 与功能检查
- 启用 `-EnforceMemoryBudgets` 时，两种模式都检查 warm-idle 的 20/24 MiB 预算、零 swap/max/PSI 压力，以及 logging-on p95 内存增量不超过 8 MiB
- 显式提供的 `-P95LimitMiB`、`-PeakLimitMiB` 和 `-ExpectedMemoryLimitMiB` 分别转发；省略的值保持未设置，不隐含 160 MiB 上限

报告列出实际生效的策略，禁用的阈值为 `null`。即使不启用内存预算，日志正确性、丢失事件和延迟门禁仍必须通过。
