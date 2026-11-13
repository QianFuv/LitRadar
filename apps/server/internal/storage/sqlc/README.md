# 生成的存储查询

本目录保存 sqlc 的 schema 快照（`schema/`）、查询输入（`queries/`）和生成的 Go 包。工具安装、生成与检查命令，以及 FTS、动态 SQL、迁移 SQL 等手写边界见[数据库参考](../../../../../docs/reference/database.md#sqlc-查询生成)；本页只说明本包内的约定。

`sqlc-schema` 在临时数据库中执行生产环境的认证、内容、Provider 控制、索引 batch 和遥测迁移，以及 Crossref 工作集初始化，再把六类数据库的关系表导出到 `schema/`；FTS 虚拟表和 SQLite shadow 表不导出。迁移实现仍是 schema 的权威来源。SQL 输入和生成的 Go 包都要提交，两类生成产物都不要手工编辑。

查询通过 `database/sql` 使用现有的 modernc SQLite 连接。生成的方法接收调用方的连接或事务，因此立即写事务、连接级 pragma、审计回滚和快照所有权都由调用方掌控。内容写入器使用 sqlc 的 prepared query 接口，并自行持有部分完成准备的语句，使准备失败时也能释放资源。其他批量循环保留各自的 prepared statement，绑定生成的 SQL 常量；这些查询的定义统一维护在 `queries/`。

类型覆盖保留严格的 SQLite 存储类与 UTF-8 校验、可空时间戳，以及 NULL 与空 BLOB 的区别。通知偏好读取保留原始 SQLite 值，直到既有的有序业务校验运行，从而保证格式错误的列表和凭据错误优先于后面格式错误的偏好列报告。
