# 备份与离线恢复

`admin backup` 提供版本化、可验证且不包含部署密钥的备份目录。所有命令都必须由操作员显式执行；应用启动、测试和数据库迁移不会自动备份或恢复真实数据。

## 备份内容与一致性

默认只备份 `data/auth.sqlite`。可选开关：

- `--include-indexes`：包含创建时发现的全部 `data/index/*.sqlite`
- `--include-push-state`：包含 `data/push_state/` 与 `data/folder_push_state/` 中的状态文件

SQLite 文件使用 online backup API，因此源数据库处于 WAL 模式且仍有已提交写入时也能得到完整的单库快照。每个数据库分别取得快照，不构成跨多个 SQLite 文件的同一事务。推送状态在复制前后各扫描并计算 hash；文件增加、删除或内容变化会使整次创建失败。需要严格的跨数据库与状态文件时间点一致性时，应先停止 API、worker 与索引/推送写入再创建备份。

部署密钥不在选择范围内，也不会写入清单。推送状态目录出现 `.key` 或 `.pem` 文件时创建会失败。数据库内的集成凭据仍是 `psenc:v1:` 密文；备份必须与对应的 32 字节密钥副本分开保存。

## 创建与验证

输出必须是不存在的新目录，并且不能放在已选择的 `data/index` 或推送状态目录内部：

```bash
admin backup create \
  --project-root /srv/paper-scanner \
  --output /srv/backups/paper-scanner-2026-07-10 \
  --include-indexes \
  --include-push-state

admin backup verify \
  --backup /srv/backups/paper-scanner-2026-07-10
```

创建命令在输出目录的同一文件系统中写入临时目录，完成清单与全部组件验证后才重命名为最终目录。`verify` 不需要部署密钥，也不修改目标数据。

每个备份根目录包含 `manifest.json`，其中记录：

- 固定格式名与格式版本
- 创建时间与可选数据组选择
- 每个文件的相对路径、类型、字节数和 SHA-256
- SQLite 组件的 `PRAGMA user_version`

验证会拒绝未知格式/版本、未来数据库 schema、缺失或额外文件、路径穿越、符号链接、特殊文件、大小/hash 不匹配以及 SQLite `quick_check` 失败。备份目录中的任何额外文件也会导致失败，因此不要在其中放说明、密钥或其他附件。

## Docker Compose

运行镜像已包含 `admin`，但根 Compose 只持久化 `/app/data`。备份必须使用独立宿主机挂载，不能写进 `/app/data`：

```bash
mkdir -p backups
sudo chown 10001:10001 backups  # 仅 Linux 原生 Docker Engine

docker compose run --rm --no-deps \
  -v "$PWD/backups:/backups:rw" \
  api admin backup create \
  --project-root /app \
  --output /backups/paper-scanner-2026-07-10 \
  --include-indexes \
  --include-push-state

docker compose run --rm --no-deps \
  -v "$PWD/backups:/backups:ro" \
  api admin backup verify \
  --backup /backups/paper-scanner-2026-07-10
```

## 离线恢复

恢复只接受已经完整验证的备份，并要求显式 `--confirm-restore`。推荐顺序：

1. 在另一份只读副本上执行 `admin backup verify`。
2. 停止 API、worker、索引与推送写入；Docker 部署可执行 `docker compose stop app api worker`。
3. 等待最近心跳超过 90 秒，或确认 API 已优雅清除自己的心跳。
4. 保留当前数据的独立回滚副本。
5. 执行恢复命令。
6. 使用与该备份匹配的密钥执行 `admin secrets verify`，然后再启动服务。

```bash
admin backup restore \
  --project-root /srv/paper-scanner \
  --backup /srv/backups/paper-scanner-2026-07-10 \
  --confirm-restore
```

Docker 示例：

```bash
docker compose run --rm --no-deps \
  -v "$PWD/backups:/backups:ro" \
  api admin backup restore \
  --project-root /app \
  --backup /backups/paper-scanner-2026-07-10 \
  --confirm-restore
```

恢复前会再次检查 `service_heartbeats` 和旧 worker 心跳；最近 90 秒内存在 API 或 worker 活动时立即拒绝。认证库以同文件系统临时文件替换，并移除旧 `-wal`、`-shm`、`-journal` 侧车；选择了索引或推送状态时，相应目标目录会被备份中的完整目录替换。任何替换或恢复后验证失败都会按逆序回滚已经应用的目标。

未选择的可选数据组不会在恢复时修改：未包含索引则保留目标 `data/index/`，未包含推送状态则保留两个目标状态目录。选择了某组则恢复其精确快照，包括“备份时为空”的情况，目标中不在清单里的旧文件会被移除。

## 失败处理

- `ActiveTarget`：服务仍在运行或心跳尚未过期；不要绕过，先停止写入并等待。
- hash、文件集或 `quick_check` 失败：备份不可用；从另一份已验证副本恢复。
- schema 版本高于当前二进制：升级到支持该版本的程序，不要手改清单或降低 `user_version`。
- 密钥不匹配：数据库文件可能恢复成功，但服务会在解密验证时失败关闭；提供与备份匹配的独立密钥副本。
- 恢复失败：保留错误输出、原备份和恢复前数据副本；不要删除临时证据或尝试手工拼接 SQLite/WAL 文件。
