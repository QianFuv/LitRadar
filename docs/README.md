# LitRadar 文档中心

这里是项目文档的统一入口。根目录 [README](../README.md) 介绍项目和最短启动路径；本页按读者任务把你引导到唯一的主题文档。

## 从这里开始

| 目标                                | 文档                                                         |
| ----------------------------------- | ------------------------------------------------------------ |
| 了解组件、数据流和持久化边界        | [系统架构](architecture/README.md)                           |
| 搭建本地开发环境                    | [开发指南](guides/development.md)                            |
| 选择测试层、命令和诊断报告          | [测试系统](guides/testing.md)                                |
| 使用 Docker Compose 部署            | [Docker 部署](operations/docker.md)                          |
| 连接 Web、Windows 或 Android 客户端 | [客户端与登录](guides/clients.md)                            |
| 查询和排查结构化日志                | [日志运维](operations/logging.md)                            |
| 查找 `litradar` 命令和参数          | [CLI 参考](reference/cli.md)                                 |
| 查找 REST API 或 MCP 行为           | [API 参考](reference/api.md)                                 |
| 理解数据库和状态文件                | [数据库参考](reference/database.md)                          |
| 接入或更换索引 Provider             | [索引与 Provider 契约](reference/index-provider-contract.md) |
| 配置文献筛选与投递                  | [通知与追踪](guides/notifications.md)                        |
| 理解征稿来源、刷新与原文保存        | [征稿追踪架构](architecture/cfp-tracking.md)                 |

## 指南

指南回答“怎样完成一项工作”。完整参数、字段和默认值链接到参考文档，不在指南中维护第二份副本。

- [客户端与登录](guides/clients.md)：Web、Windows x64、Android arm64-v8a、服务端 URL 与设备令牌
- [开发指南](guides/development.md)：工具链、开发流程、OpenAPI 类型生成、sqlc 与 golangci-lint
- [测试系统](guides/testing.md)：五层测试模型、功能所有权、共享场景、统一命令、报告和 flaky/覆盖率策略
- [性能剖析](guides/profiling.md)：Go 镜像画像、Docker 内存画像与门禁、日志开销对照
- [通知与追踪](guides/notifications.md)：候选来源、AI 选择、PushPlus、追踪文件夹、状态和排障

## 运维

运维文档面向部署和维护人员。安全敏感操作以相应运维文档为准，其他文档只给出入口链接。

- [Docker 部署](operations/docker.md)：镜像、Compose、权限、健康检查、安全边界和故障排查
- [安全说明](operations/security.md)：部署密钥、凭据加密、管理员初始化、密码、限流和网络暴露
- [authentik 单点登录](operations/authentik.md)：authentik 提供程序、LitRadar 运行设置、账号建立与绑定、回调日志
- [备份与恢复](operations/backup.md)：创建、验证、离线恢复、回滚和失败处理
- [日志运维](operations/logging.md)：事件契约、级别、关联、保留、查询、隐私和事故处理
- [版本发布](operations/releases.md)：版本规则、发布流水线、二进制包安装和失败重试（随发行包分发）
- [CI 执行](operations/ci.md)：工作流阶段、质量 job、缓存与产物归属

## 参考

参考文档回答“系统当前是什么”。请求/响应 schema 以由 Go 类型生成的 OpenAPI 为准，Markdown 只补充跨接口和业务语义。

- [API 参考](reference/api.md)：认证、数据库选择、分页、缓存、端点目录、MCP 和业务约束
- [CLI 参考](reference/cli.md)：`litradar` 的 `serve`、`admin`、`index`、`cfp`、`notify`、`push`、`scheduler` 和 `openapi` 子命令
- [运行配置](reference/configuration.md)：配置来源、全局运行设置、密钥文件和固定路径
- [数据库参考](reference/database.md)：当前 schema、表关系、版本和外部状态文件
- [索引与 Provider 契约](reference/index-provider-contract.md)：规范期刊/文章模型、稳定身份、可选在线能力和 Provider 更换流程
- [Scholarly 数据源](reference/sources/scholarly.md)：Crossref、OpenAlex、Semantic Scholar
- [CNKI 数据源](reference/sources/cnki.md)：CNKI 国内接口、验证码和浙江图书馆全文边界
- [前端设计系统](reference/design-system.md)：字体、StyleX 主题 token、基础组件变体、动效、响应式与无障碍约定

## 架构专题

[系统架构](architecture/README.md)说明进程、模块和数据流；[征稿追踪架构](architecture/cfp-tracking.md)进一步说明 CFP 来源、证据保存、刷新边界与测试。导入和刷新步骤以 [CLI 参考](reference/cli.md#cfp)为准。

## 包级文档

- [客户端包说明](../apps/client/README.md)：`apps/client/` 的 Wails 构建任务、版本派生、原生宿主与冒烟测试
- [前端包说明](../apps/client/frontend/README.md)：`apps/client/frontend/` 的工具链、启动、目录、API 契约和质量检查
- [sqlc 生成查询](../apps/server/internal/storage/sqlc/README.md)：schema 快照、查询生成与检查命令
- [严格文本解码数据](../apps/server/internal/platform/textdecode/COMPATIBILITY.md)：GB18030/GBK 修正表来源与校验值
- [日志过滤 Unicode 16 数据表](../apps/server/internal/runtime/logfilter/unicode16/README.md)：内嵌 Unicode 表的上游与哈希
- [go-sdk 补丁](../third_party/go-sdk-patches/README.md)：vendored MCP SDK 的本地补丁、校验与升级流程

`docs/third-party/` 保存随镜像和发行包分发的第三方许可证与固定发行记录，不是阅读文档。

## 事实来源

文档之间或文档与实现不一致时，按以下顺序判断：

1. 当前实现、配置和生成产物
2. 对应测试与 CI 工作流
3. 本文档体系中的主题所有者
4. 根 README 和示例

主要映射：

| 事实               | 实现来源                                                                                                                                                         | 文档所有者                                                   |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| 工具链版本         | `go.mod`、根 `package.json`、`.github/workflows/`                                                                                                                | [开发指南](guides/development.md#工具链)                     |
| 进程与服务生命周期 | `apps/server/internal/runtime/`                                                                                                                                  | [系统架构](architecture/README.md)                           |
| CLI 参数和默认值   | `apps/server/internal/cli/`                                                                                                                                      | [CLI 参考](reference/cli.md)                                 |
| 全局运行配置       | `apps/server/internal/storage/settings/`                                                                                                                         | [运行配置](reference/configuration.md)                       |
| REST schema        | `apps/server/internal/api/` 的 huma 操作类型、`apps/server/internal/openapi/`；生成基线 `apps/client/frontend/lib/generated/openapi.json`                        | OpenAPI；[API 参考](reference/api.md)补充语义                |
| SQLite schema      | `apps/server/internal/storage/migrations/`、`apps/server/internal/storage/indexschema/`；sqlc 输入 `apps/server/sqlc.yaml`、`apps/server/internal/storage/sqlc/` | [数据库参考](reference/database.md)                          |
| Provider 内容契约  | `apps/server/internal/domain/sources/`、`apps/server/internal/provider/`                                                                                         | [索引与 Provider 契约](reference/index-provider-contract.md) |
| Docker 行为        | `apps/server/Dockerfile`、`docker-compose.yml`、`Caddyfile`                                                                                                      | [Docker 部署](operations/docker.md)                          |
| 结构化日志         | `apps/server/internal/runtime/observability/`、`apps/client/frontend/lib/client-logger.tsx`                                                                      | [日志运维](operations/logging.md)                            |
| 版本与发布产物     | `VERSION`、`.github/workflows/release.yaml`、`apps/server/scripts/`                                                                                              | [版本发布](operations/releases.md)                           |
| CI 阶段与缓存      | `.github/workflows/`                                                                                                                                             | [CI 执行](operations/ci.md)                                  |
| 原生客户端宿主     | `apps/client/`、`apps/client/Taskfile.yml`                                                                                                                       | [客户端包说明](../apps/client/README.md)                     |
| 前端结构           | `apps/client/frontend/package.json`、`vite.config.ts`、`src/`、`lib/`、`components/`                                                                             | [前端包说明](../apps/client/frontend/README.md)              |
| 测试分层与诊断     | `tests/test.mjs`、`apps/client/frontend/vite.config.ts` 的 `test` 块、Playwright 配置                                                                            | [测试系统](guides/testing.md)                                |
| UI token 与组件    | `apps/client/frontend/lib/theme.stylex.tsx`、`src/globals.css`、`components/ui/`                                                                                 | [前端设计系统](reference/design-system.md)                   |

## 维护原则

编辑文档时，优先保留技术事实、适用条件和代码语法，再调整段落、术语与排版；README、指南和参考文档分别围绕各自的读者任务组织内容。

- 所有对外文档使用中文；代码标识符、命令、路径和错误字符串保持原样。
- 一个事实只保留一个完整说明，其他文档使用链接。
- 示例必须能由当前命令、路由、schema 或配置验证。
- 只描述当前行为，不把迁移历史写成当前架构；仅保留仍需执行的兼容或运维步骤。
- 外部 API 文档说明上游规则，数据源文档另外说明仓库实际请求策略。
- 发现实现疑似错误时，明确标记风险，不通过修改文档掩盖差异。
- `docs/operations/releases.md` 会作为发行包内的 `README.md` 分发，其中链接使用 GitHub 绝对地址。
