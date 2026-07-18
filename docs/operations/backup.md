# 备份与恢复

`litradar admin backup` 创建版本化、可独立验证的目录备份。部署密钥永远不在备份中，必须单独保存。命令语法见 [CLI 参考](../reference/cli.md)。

## 备份范围

当前创建格式为 v2。每个 v2 备份始终包含 `data/auth.sqlite` 和 `data/meta` 的完整普通文件树，包括清单管理的 CSV、用户新增文件和嵌套文件；这两组不受可选 flag 控制：

| 范围或选项             | 内容                                            |
| ---------------------- | ----------------------------------------------- |
| v2 固定范围            | `data/auth.sqlite` 和完整 `data/meta`           |
| `--include-indexes`    | 创建时发现的全部 `data/index/*.sqlite`          |
| `--include-push-state` | `data/push_state/` 和 `data/folder_push_state/` |
| 始终排除               | `data/index-control/` Provider checkpoint/lease |

未选择的索引和状态组不会在恢复时修改。v2 Meta 和选择的可选组按精确快照恢复，包括“备份时为空”的情况。

`--include-indexes` 只选择 Provider-neutral v4 内容库。`data/index-control` 是可重建的运行控制状态，即使目录位于项目根下也不会进入 manifest、备份树或恢复目标。Provider 切换不需要复制旧 checkpoint；恢复后首次索引会重新创建控制库，并通过内容 identity alias 幂等收敛。

部署密钥不在选择范围内。Meta 或状态目录中出现 `.key` 或 `.pem` 文件时，创建会失败。

## 一致性

SQLite 使用 online backup API，可在 WAL 数据库仍有已提交写入时获得单库一致快照。限制：

- 创建 v2 备份时，认证库的 `BEGIN IMMEDIATE` 写锁覆盖认证库快照和 Meta 目录复制；Meta 在复制前后扫描文件集、大小和 hash，确保受管状态与复制期间稳定的目录一起捕获
- 多个 SQLite 文件之间不是同一事务
- 索引数据库和 push 状态与认证库/Meta 快照之间不是同一事务
- 控制库不参与快照；活跃索引仍可能向内容库提交，因此严格时间点备份应停止索引命令
- Meta 和状态目录在复制前后各扫描并计算 hash；期间新增、删除或变化会使创建失败

需要严格时间点一致性时，先停止 `litradar serve`，并确认没有独立的索引或投递子命令仍在写入。

## 创建

输出必须是不存在的新目录，且不能位于被选择的索引或状态目录内部：

```bash
litradar admin backup create \
  --project-root /srv/litradar \
  --output /srv/backups/litradar-2026-07-10 \
  --include-indexes \
  --include-push-state
```

命令在输出目录的同一文件系统中构建临时目录，完成内部验证后才重命名为最终目录。

## 清单和验证

每个备份根目录含 `manifest.json`：

- 固定格式名和版本
- 创建时间
- 已选择的数据组
- 每个文件的相对路径、类型、字节数和 SHA-256
- SQLite `PRAGMA user_version`

清单格式名固定为 `litradar-backup`。当前二进制创建 version 2，也接受规范的 version 1 清单。v1 没有 Meta 选择和 `meta/` 组件；恢复 v1 时目标 `data/meta` 完全保持原样。改名前的其他格式仍会被拒绝，不提供别名识别或自动转换。

独立验证不需要部署密钥，也不修改数据：

```bash
litradar admin backup verify \
  --backup /srv/backups/litradar-2026-07-10
```

验证拒绝：

- 未知备份格式或版本
- 高于当前二进制支持的数据库 schema
- 缺失、额外或重复文件
- 路径穿越、符号链接和特殊文件
- 大小或 SHA-256 不符
- SQLite `quick_check` 失败

备份目录中的任何额外说明或附件也会失败。把说明和密钥放在备份目录之外。

## Docker Compose

唯一的 `litradar` 服务只有 `/app/data` 持久挂载，备份应使用独立 `/backups`：

```bash
mkdir -p backups
sudo chown 10001:10001 backups

docker compose run --rm --no-deps \
  -v "$PWD/backups:/backups:rw" \
  litradar admin backup create \
    --project-root /app \
    --output /backups/litradar-2026-07-10 \
    --include-indexes \
    --include-push-state

docker compose run --rm --no-deps \
  -v "$PWD/backups:/backups:ro" \
  litradar admin backup verify \
    --backup /backups/litradar-2026-07-10
```

`chown` 只适用于 Linux 原生 Docker Engine。

## 离线恢复

推荐顺序：

1. 在只读副本上再次运行 `litradar admin backup verify`。
2. 停止所有读写目标数据的服务。
3. 等待统一应用记录的服务与调度心跳都超过 90 秒。
4. 保留当前目标数据的独立回滚副本。
5. 执行带显式确认的恢复。
6. 用与备份匹配的部署密钥运行 `litradar admin secrets verify`。
7. 启动服务并通过同一 8000 入口检查 Web、`/health/live` 和 `/health/ready`。

本机：

```bash
litradar admin backup restore \
  --project-root /srv/litradar \
  --backup /srv/backups/litradar-2026-07-10 \
  --confirm-restore
```

Docker：

```bash
docker compose stop litradar

docker compose run --rm --no-deps \
  -v "$PWD/backups:/backups:ro" \
  litradar admin backup restore \
    --project-root /app \
    --backup /backups/litradar-2026-07-10 \
    --confirm-restore
```

不要绕过 `--confirm-restore` 或活动心跳检查。

## 替换和回滚

恢复开始前会再次验证整个备份，并在替换前后检查 `service_heartbeats` 和 `scheduler_workers`。这些表名是统一进程内部 HTTP 与调度组件的持久化活动记录，不表示独立服务。

认证库使用同文件系统临时文件替换，并清理旧 `-wal`、`-shm`、`-journal`。v2 的 Meta 目录以及选择的索引或状态组会整体替换；目标中不在备份清单内的 Meta 文件也会删除。任何替换或恢复后验证失败都会按逆序回滚已应用目标。

恢复 v1 时目标 Meta 目录保持原样。未选择的索引或状态组也保持原样；选择的组会移除目标中不在清单内的旧文件。

恢复不会创建、替换或清理目标 `data/index-control`。为避免把恢复前 checkpoint 与恢复后的内容时间点混用，离线恢复包含索引库时，应在确认没有索引进程后删除明确对应的 control 文件，让下一次索引从空控制状态重建；该删除不会影响内容 ID。

## 失败处理

| 失败                           | 处理                                       |
| ------------------------------ | ------------------------------------------ |
| `ActiveTarget`                 | 服务或心跳仍活跃；停止写入并等待，不要绕过 |
| hash/文件集/`quick_check` 失败 | 备份不可用，改用另一份已验证副本           |
| schema 版本过高                | 升级应用，不修改清单或降低 `user_version`  |
| 密钥不匹配                     | 提供与该数据库备份匹配的独立密钥           |
| 替换失败                       | 保留错误输出、原备份、恢复前副本和临时证据 |

不要手工拼接 SQLite、WAL 或状态目录来“修复”失败恢复。
