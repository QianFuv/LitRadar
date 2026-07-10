# Paper Scanner 文档中心

这里是项目文档的统一入口。根目录 [README](../README.md) 负责介绍项目和最短启动路径；本页负责把不同读者引导到唯一的主题文档。

## 从这里开始

| 目标                         | 文档                                |
| ---------------------------- | ----------------------------------- |
| 了解组件、数据流和持久化边界 | [系统架构](architecture.md)         |
| 搭建本地开发环境             | [开发指南](guides/development.md)   |
| 使用 Docker Compose 部署     | [Docker 部署](operations/docker.md) |
| 查找 Rust 命令和参数         | [CLI 参考](reference/cli.md)        |
| 查找 REST API 或 MCP 行为    | [API 参考](reference/api.md)        |
| 理解数据库和状态文件         | [数据库参考](reference/database.md) |

## 指南

- [开发指南](guides/development.md)：本地环境、开发流程、OpenAPI 类型生成、测试和质量检查
- [通知与追踪](guides/notifications.md)：候选来源、AI 选择、PushPlus、追踪文件夹、状态和排障

指南回答“怎样完成一项工作”。完整参数、字段和默认值应链接到参考文档，不在指南中维护第二份副本。

## 运维

- [Docker 部署](operations/docker.md)：镜像、Compose、权限、健康检查、生产边界和故障排查
- [安全说明](operations/security.md)：部署密钥、凭据加密、管理员初始化、密码、限流和网络暴露
- [备份与恢复](operations/backup.md)：创建、验证、离线恢复、回滚和失败处理

运维文档面向部署和维护人员。安全敏感操作以相应运维文档为准，其他文档只给出入口链接。

## 参考

- [API 参考](reference/api.md)：认证、数据库选择、分页、缓存、端点目录、MCP 和业务约束
- [CLI 参考](reference/cli.md)：`admin`、`api`、`index`、`notify`、`push`、`scheduler`、`worker`
- [运行配置](reference/configuration.md)：配置层次、七个全局运行设置、密钥文件和前端变量
- [数据库参考](reference/database.md)：当前 schema、表关系、迁移版本和外部状态文件
- [Scholarly 数据源](reference/sources/scholarly.md)：Crossref、OpenAlex、Semantic Scholar
- [CNKI 数据源](reference/sources/cnki.md)：CNKI overseas 元数据和浙江图书馆全文边界
- [前端设计系统](reference/design-system.md)：字体、主题 token、组件 variants、响应式与无障碍约定

参考文档回答“系统当前是什么”。请求/响应 schema 以运行时生成的 OpenAPI 为准，Markdown 只补充跨接口和业务语义。

## 包级文档

- [前端包说明](../app/README.md)：`app/` 的启动、目录、API 契约和测试
- [simple 分词器](../libs/simple/README.md)：仓库内置扩展、发现规则和上游许可证

`libs/simple-*/**/dict/README.md` 是第三方词典说明，不属于 Paper Scanner 的项目文档，保持上游内容。

## 事实来源

当文档之间或文档与实现不一致时，按以下顺序判断：

1. 当前实现、配置和生成产物
2. 对应测试与 CI 工作流
3. 本文档体系中的主题所有者
4. 根 README 和示例

主要映射：

| 事实             | 实现来源                                                               | 文档所有者                                    |
| ---------------- | ---------------------------------------------------------------------- | --------------------------------------------- |
| CLI 参数和默认值 | `crates/ps-cli/src/lib.rs`、`crates/ps-api/src/config.rs`              | [CLI 参考](reference/cli.md)                  |
| 全局运行配置     | `crates/ps-storage/src/business/runtime_settings.rs`                   | [运行配置](reference/configuration.md)        |
| REST schema      | `app/lib/generated/openapi.json`                                       | OpenAPI；[API 参考](reference/api.md)补充语义 |
| SQLite schema    | `crates/ps-storage/src/migrations.rs`、`crates/ps-index/src/schema.rs` | [数据库参考](reference/database.md)           |
| Docker 行为      | `Dockerfile`、`app/Dockerfile`、`docker-compose.yml`                   | [Docker 部署](operations/docker.md)           |
| 前端结构         | `app/package.json`、`app/app/`、`app/lib/`、`app/components/`          | [前端包说明](../app/README.md)                |
| UI token 与组件  | `app/app/globals.css`、`app/components/ui/`                            | [前端设计系统](reference/design-system.md)    |

## 维护原则

- 一个事实只保留一个完整说明，其他文档使用链接。
- 示例必须能由当前命令、路由、schema 或配置验证。
- 不把迁移历史写成当前架构；仅保留仍需执行的兼容或运维步骤。
- 外部 API 文档说明上游规则，数据源文档另外说明仓库实际请求策略。
- 发现实现疑似错误时，明确标记风险，不通过修改文档掩盖差异。
