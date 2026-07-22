# 日志运维

本文档是 LitRadar 服务端和浏览器错误日志的唯一运维说明。它定义当前日志契约、配置、关联方式、保留边界、隐私规则、查询方法和事故处理流程。进程与数据流见[系统架构](../architecture.md)，日志字段的安全边界同时受[安全说明](security.md)约束。

## 输出边界

LitRadar 只有一个进程级 tracing subscriber：

- 服务端事件写入 `stderr`，默认每行一个 JSON 对象；CLI 的业务结果继续独占 `stdout`。
- Docker 不在应用文件系统中写日志。Compose 收集容器 `stderr`，应用根文件系统仍为只读。
- 调度启动的 `litradar` 子进程继承父进程 `stderr`，因此事件实时进入同一容器日志。
- 浏览器只把经过白名单裁剪的错误对象写入当前浏览器开发者控制台，不上传到服务端，也不写入 Web Storage。

需要把 CLI 结果交给其他程序时，应分别处理两个流：

```bash
litradar openapi > openapi.json 2> litradar.log
```

不要把 `stderr` 合并进待解析的 CLI JSON、OpenAPI 或其他业务输出。

## 服务端事件契约

生产默认格式是 JSON Lines。每个正常 tracing 事件至少包含：

| 字段          | 含义                                                                  |
| ------------- | --------------------------------------------------------------------- |
| `timestamp`   | UTC RFC 3339 时间                                                     |
| `level`       | `TRACE`、`DEBUG`、`INFO`、`WARN` 或 `ERROR`                           |
| `target`      | 产生事件的 Rust target                                                |
| `event`       | 稳定的点分事件名，例如 `http.request.completed`                       |
| `component`   | 稳定组件名，例如 `runtime`、`http`、`index`、`scheduler`、`storage`   |
| `outcome`     | 适用时的有限结果枚举，例如 `success`、`failure`、`client_error`       |
| `error_kind`  | 适用时的安全错误分类；不包含底层错误消息                              |
| `duration_ms` | 适用时的整毫秒耗时；消费者应接受 JSON 数字或非负十进制字符串          |
| `span`        | 当前关联上下文；HTTP 请求含服务端生成的 `request_id`、method 和 route |
| `spans`       | 从进程到当前工作的有序上下文链                                        |

事件专有字段只允许使用计数、状态码、有限枚举、安全内部 ID 和有界关联 ID。消费者必须按字段名读取，并容忍未来增加字段；不能依赖 JSON 键顺序。

典型生产事件：

```json
{
  "timestamp": "2026-07-18T03:20:41.152Z",
  "level": "WARN",
  "event": "http.request.completed",
  "component": "http",
  "request_id": "019be8c4-4e8a-7c21-9d17-9017ca04a290",
  "method": "GET",
  "route": "api.unmatched",
  "status": 404,
  "outcome": "client_error",
  "duration_ms": 1,
  "target": "litradar_api::http_observability",
  "span": {
    "component": "http",
    "request_id": "019be8c4-4e8a-7c21-9d17-9017ca04a290",
    "method": "GET",
    "route": "api.unmatched",
    "name": "http.request"
  },
  "spans": [
    {
      "component": "http",
      "request_id": "019be8c4-4e8a-7c21-9d17-9017ca04a290",
      "method": "GET",
      "route": "api.unmatched",
      "name": "http.request"
    }
  ]
}
```

`event` 使用 `<domain>.<subject>.<state>` 风格。主要域包括：

- `process.*`、`service.*`：进程和组合服务生命周期
- `http.request.*`：请求终态和安全的 API 错误分类
- `index.*`、`source.*`：索引、CSV、worker 与上游请求汇总
- `scheduler.*`：tick、认领、运行和子进程生命周期
- `delivery.*`、`ai.*`、`pushplus.*`：投递工作流与外部请求汇总
- `storage.*`：迁移、备份和受管 Meta 准备
- `security.auth.*`、`security.admin.*`：认证结果、限流和状态变更审计
- `logging.events_dropped`：非阻塞日志队列在过载时丢失的行数

## 级别规则

| 级别    | 使用场景                                                      |
| ------- | ------------------------------------------------------------- |
| `TRACE` | 默认关闭；仅用于临时、无敏感字段的极细粒度诊断                |
| `DEBUG` | 默认关闭；安全的尝试级或分支级诊断                            |
| `INFO`  | 启停、成功的工作流终态、重定向和重要状态变化                  |
| `WARN`  | 客户端错误、限流、可恢复降级、fallback 和未终止进程的异常情况 |
| `ERROR` | 5xx、工作流失败、组件失败、进程失败和 panic                   |

成功的 `/health/live`、`/health/ready`、静态资源和前端文档请求不产生逐请求完成事件；非成功响应仍记录。该抑制避免健康检查和静态流量淹没有效信号。

## 配置

日志配置是 `data/auth.sqlite.runtime_settings` 中的两个非秘密管理员设置：

| 字段         | 默认值 | 前端控件 | 生效方式 | 允许值/语义                                  |
| ------------ | ------ | -------- | -------- | -------------------------------------------- |
| `log_format` | `json` | 单选     | 重启进程 | `json` 或 `compact`                          |
| `log_filter` | 见下文 | 文本     | 重启进程 | tracing `EnvFilter` 指令；`off` 完全关闭事件 |

默认 filter 为：

```text
warn,litradar=info,litradar_api=info,litradar_cli=info,litradar_index=info,litradar_sources=info,litradar_storage=info,litradar_worker=info
```

管理员页面从后端运行设置元数据选择控件、允许值和“重启后生效”提示。保存时 API 严格验证 format 和 filter，并与同一请求中的其他设置原子提交。每个 `litradar` 进程在迁移或子命令分发前，按 `--project-root`/`--auth-db` 解析认证库并只读加载这两行；数据库或表不存在时使用默认值。已存在但非法或不可读的配置让进程在业务工作前失败，不会静默回退，也没有环境变量兼容别名。

本地交互开发推荐 compact：

1. 先以默认 JSON 启动服务并登录管理员页面。
2. 在“运行配置 → 可观测性”把“Log format”改为 `compact`。
3. 保存后重启当前服务或下一条 CLI 进程。

compact 只改变显示形式，不改变事件选择或隐私规则。一次本地命令看起来类似：

```text
2026-07-18T03:22:14.681Z  INFO process: litradar: event="process.started" component="runtime" component="runtime" command="help" version="0.1.0" process_id=42412
2026-07-18T03:22:14.682Z  INFO process: litradar: event="process.completed" component="runtime" outcome="success" duration_ms=0 component="runtime" command="help" version="0.1.0" process_id=42412
```

生产和需要机器解析的本地检查保持默认 JSON。临时增加目标级别时，在同一管理分组把 `log_filter` 保存为最窄指令，例如 `warn,litradar=debug,litradar_api=debug`，重启并完成诊断后再恢复默认值。

即使启用 `DEBUG` 或 `TRACE`，也禁止增加秘密或内容字段。

## 关联

### HTTP

服务器会删除客户端提供的 `X-Request-Id`，为每个请求生成新的 UUID，并在响应 `X-Request-Id` 与请求 span 中返回同一个值。日志使用匹配路由模板或固定分类，不记录原始 URL、query 或 fragment。

非 2xx API 响应在浏览器中形成 `ApiError.requestId`。若该错误到达前端错误边界，本地 `client.error` 对象可带同一个 `request_id`；运维人员可让用户提供该 ID，再查询服务端事件。跨源浏览器访问时，后端只暴露生成的 `X-Request-Id` 响应头。

### 调度与子进程

调度事件在 span 中包含安全的 `task_id`、`job_id`、`run_id` 和 `worker_id`。父进程通过隐藏内部 argv 把当前 run ID 传给规范子进程；应用在公共命令分发前验证并移除它，子进程的 process span 记录 `parent_run_id`。该值最长 128 字节，首字符必须为 ASCII 字母或数字，其余只允许 ASCII 字母、数字和 `-_.`。参数不出现在 `--help`，也不属于用户或管理员配置。

索引 worker 继续使用索引 `run_id` 和 worker 标识关联。不要用命令参数文本、文件完整路径或凭据建立关联。

## 浏览器本地事件

浏览器错误记录器只调用 `console.error`，不会发送 fetch、beacon 或其他遥测请求。固定字段为：

```json
{
  "timestamp": "2026-07-18T03:24:05.810Z",
  "level": "error",
  "event": "client.error",
  "component": "browser",
  "source": "route_boundary",
  "route": "/admin",
  "error_kind": "api_error",
  "request_id": "019be8c4-4e8a-7c21-9d17-9017ca04a290"
}
```

`source` 只允许 `route_boundary`、`global_boundary`、`window_error` 或 `unhandled_rejection`。可选 `digest` 和 `request_id` 必须是最长 128 个字符的安全标识；`route` 只保留 pathname。记录器不枚举原错误，不包含 message、stack、promise reason、query、请求/响应 body、token 或浏览器存储。相同 Error 对象只报告一次，React Strict Mode 的 listener 会成对安装和清理。

## 队列与丢失语义

服务端使用 4096 行有界、lossy、非阻塞队列。业务线程在日志消费者变慢时继续运行，代价是过载事件可能被丢弃。正常关闭先排空队列；若发生丢失，随后直接向 `stderr` 写一条：

```json
{
  "level": "WARN",
  "target": "litradar",
  "event": "logging.events_dropped",
  "component": "logging",
  "dropped_count": 37
}
```

该关闭兜底事件不经过已经关闭的队列，因此只保证上述字段。`SIGKILL`、OOM、宿主机崩溃或无法写入 `stderr` 时，进程没有机会报告精确丢失数；此时应结合 Docker 状态、时间缺口和上游事件判断。正常预期负载的门禁是 `dropped_count=0`。

## Docker 保留策略

根 Compose 使用 Docker `local` 日志驱动：

```yaml
logging:
  driver: local
  options:
    max-size: 10m
    max-file: "5"
    compress: "true"
```

每个容器最多保留五个、每个轮转阈值 10 MiB 的驱动文件，即压缩效果之前约 50 MiB 的明确上界；实际磁盘占用受记录边界、元数据和压缩率影响。这个策略属于 Docker 宿主机，不会给只读容器根文件系统增加写路径。删除容器也会删除该容器的驱动日志，因此需要长期审计时必须在保留窗口内导出到受控系统。

不要直接读取 Docker 内部日志文件。使用 Docker CLI：

```bash
docker compose logs --since 30m --timestamps litradar
docker compose logs --no-log-prefix litradar | jq -c 'select(.level == "ERROR")'
```

按请求 ID 查询：

```bash
request_id='019be8c4-4e8a-7c21-9d17-9017ca04a290'
docker compose logs --no-log-prefix litradar |
  jq -c --arg id "$request_id" 'select(.request_id == $id or .span.request_id == $id)'
```

PowerShell：

```powershell
docker compose logs --no-log-prefix litradar |
  ForEach-Object { $_ | ConvertFrom-Json } |
  Where-Object { $_.level -eq "ERROR" }
```

## 性能画像

先构建当前镜像，并对隔离数据副本运行三轮 off/on 对照：

```powershell
docker compose build
pwsh ./scripts/profile_logging.ps1 `
  -DataPath ./output/logging-fixture `
  -Rounds 3 `
  -RequestCount 300 `
  -Concurrency 4
```

脚本先用正常容器启动完成迁移，通过可配置的 `sqlite3` CLI 快照 `log_format`/`log_filter`，再事务性切换 off/default 两种模式并在结束时恢复原始行。它交错运行两种模式，对 `/api/logging-profile-missing` 发起固定 404 请求，验证每个默认模式应用行都是 JSON 且含必填字段，并检查请求事件数与丢失数。它还分别调用现有 `profile_docker_memory.ps1` 的 warm-idle 场景，复用 20 MiB p95、24 MiB peak 和 160 MiB cgroup 门禁。报告写入已忽略的 `output/logging/`。

门禁：

- 预期负载 `dropped_count=0`
- logging-on p95 延迟增量不超过 `max(2 ms, logging-off p95 × 15%)`
- 两种模式都通过 20/24 MiB warm-idle 门禁
- logging-on warm-idle p95 比 logging-off 最多增加 8 MiB

只使用隔离 fixture。脚本会启动迁移并读写传入的数据目录；不要把正在运行或未备份的生产 `data/` 交给画像脚本。

## 隐私规则

任何级别都不得记录：密码、部署密钥、Cookie、Bearer token、邀请码、访问令牌、API key、PushPlus token、认证头、用户输入的用户名/邮箱、请求或响应 body、URL query、文章标题/摘要、公告内容、AI prompt/response、会话 JSON、密文、hash 或完整文件路径。

允许字段限于：事件/组件/命令/阶段枚举、HTTP method/匹配路由/status、耗时与计数、provider/source 名、数据库种类和受控逻辑名、安全内部数值 ID、UUID/run ID、布尔状态及安全错误分类。新增字段前必须用唯一 sentinel 在成功和失败路径验证日志中不存在秘密与内容。

## 事故处理

1. 记录事故时间窗口、部署版本、容器状态和用户提供的 `X-Request-Id`；不要索要密码、token 或请求 body。
2. 用 `docker compose logs --since/--until --no-log-prefix` 导出最小时间窗口，保持原始 JSON Lines。
3. 先按 `request_id`、`run_id`、`parent_run_id`、`event` 和 `component` 关联，再按 `outcome`、`error_kind` 和状态码缩小范围。
4. 检查 `logging.events_dropped`、容器 OOM/重启和时间缺口。存在丢失时，不把“没有某事件”解释为业务未发生。
5. 分享日志前再次扫描秘密和内容 sentinel；若发现禁止字段，立即按凭据泄漏处理并轮换受影响秘密。
6. 在保留窗口内保存必要证据，问题解决后按组织策略删除导出副本。不要通过提高日志保留量或启用更宽 filter 永久绕过隐私边界。
