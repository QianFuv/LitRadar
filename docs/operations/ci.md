# CI 执行

CI 在 `main` 推送和 pull request 上运行全部质量门禁。版本递增和显式的版本重试另外运行打包、平台冒烟测试和发布。定时覆盖率只作参考。不使用路径过滤，也不用缓存的测试结果替代任何必需门禁。版本规则、发布产物和重试步骤见[版本发布](releases.md)。

## 共享工作流约定

全部工作流 YAML 使用两空格结构缩进和同一套阶段词汇：`detect-changes` -> `quality` -> `build-acceptance` -> `publish`。工作流名称、job ID 以及显式或隐式的 job 名称是必需检查的身份，不要重命名；阶段标签和 summary 只用于统一展示流程。变更检测按整个推送判断版本变化，不过滤路径，也不跳过必需的质量检查。普通推送运行质量检查及其生产前端构建；发布打包只在版本递增或显式同版本重试时运行。

每个传输的产物都带源码 SHA、目标平台和文件 SHA-256 证据。通用 `ci-manifest.json` schema 记录 `sourceSha`、`platforms`、`stage`、`status`、检查结果/耗时、缺失路径和相对于产物的文件 hash。与平台无关的导出标为 `independent`；合并的发布证据列出两个目标平台。大型镜像 tar 以流方式计算 hash。清单保存在现有产物根目录内，位于正式 Release 的 `assets/` 之外，因此不改变下载布局和版本化的服务端/客户端公开资产。客户端证据还绑定其镜像 tar 和可选的已签名 Android 包。

成功的构建传输在上传前必须有实际负载，只有清单不能满足约定。诊断报告可以在更早的失败后记录缺失文件。最终 summary 在上传之后运行并报告当前 job 状态，因此上传失败不会显示为成功。所有阶段使用相同的标题、源码/平台字段和检查/状态/耗时表；前端的耗时与 flaky 测试策略以及覆盖率的参考性策略保持可见。

## 质量 job

- `Release rules` 先对推送分类，再启动后端、前端和客户端工作流。
- 后端 job 在 Linux 和 Windows 上分别运行独立的 `regular` 与 `race` 通道。`regular` 通道负责格式、模块/依赖校验、显式 vet、完整根模块测试和替换模块的常规测试；`race` 通道运行完整的根模块与替换模块 race 测试。`regular` 通道以 `CGO_ENABLED=0` 构建，不需要 C 编译器；`race` 通道使用 `CGO_ENABLED=1`（Go race detector 需要 cgo）并记录编译器身份。两者都记录诊断。集成/SQLite/进程测试都在完整 Go 套件中，两个通道都不使用 `-short`。
- `regular` 通道还使用根 `.golangci.yml`（决定启用的 linter）运行 `golangci-lint run . ./apps/server/...`，用 `go run ./apps/server/cmd/sqlc-schema -check -output apps/server/internal/storage/sqlc/schema` 检查由迁移派生的 sqlc schema 快照，并运行 `sqlc diff -f apps/server/sqlc.yaml`。每项都在 `test-results/go/` 下写日志。`race` 通道不安装这些工具。
- 客户端 job 安装同一固定版本的 golangci-lint，并在前端 dist 构建后执行 lint。Web 使用 `server,production` 构建标签；Windows 同时检查 `production` 和 `production,smoke`；Android 使用 `android,production`、构建所用 NDK 编译器、arm64 目标和 cgo。报告随客户端证据上传。有意省略的 Android 发布包同时省略其目标 lint，与包的构建门禁一致。
- 两个 `Backend / Go checks (<os>)` 名称是聚合门禁，各自要求整个四 job 矩阵成功；失败、取消或跳过都不能成为通过的发布前置条件。
- 前端契约生成检查、`pnpm check`（Vite+ `vp check`：Oxfmt 格式、Oxlint 和 TypeScript 7 类型检查）、Knip、jsdom、浏览器组件、fixture 流程和生产全栈流程都是必需的。
- 客户端工作流（`Client`）运行 Web linux/amd64、Windows x64 和 Android arm64 质量通道，并独占 Web host 检查；`frontend.yaml` 负责 Vite 契约、静态检查和浏览器流程。打包工具版本、Android SDK/NDK 和签名要求见[版本发布](releases.md#客户端打包)。
- 只有普通质量运行共享 PR/ref 并发组。带发布的运行使用包含 run ID 的独立组，因此之后的普通推送不能取消某个版本的前置检查。发布保留版本/`latest` 的串行化，不存在外层的分支级发布取消。

`node apps/server/scripts/check-go.mjs` 在本地运行全部 Go 检查；传入 `regular` 或 `race` 可复现单个 CI 通道。不要在同一工作目录中并发运行两个通道，它们共享诊断路径；托管矩阵 job 使用各自的 checkout。

## 缓存与产物归属

- Go 模块缓存显式包含根模块和 go-sdk 本地替换模块的 `go.mod`/`go.sum`。
- 编译缓存按 OS、架构、精确 Go 工具链、通道以及 `race` 通道的编译器身份分区；最终 key 包含源码提交。兼容前缀恢复可以加速新提交，新提交随后保存自己的完整缓存，而不是在依赖 key 不变时冻结首次构建。Go 仍在内部校验源码输入。
- `regular` 通道缓存 golangci-lint 和 sqlc 二进制，key 由 OS、架构、Go 工具链和两个固定工具版本（`backend.yaml` 中的 `GOLANGCI_LINT_VERSION` 和 `SQLC_VERSION`）组成。
- pnpm 缓存以根 lockfile 为 key；Vite 和 pnpm 缓存与测试/构建产物分开。
- BuildKit 使用 `litradar-linux-amd64` cache scope，并通过自身缓存图对 Dockerfile、构建上下文、参数和固定基础镜像做 hash。

测试保留 `-count=1`。只有根模块常规测试在相同配置的显式 `go vet` 成功后禁用隐式 vet；race 选中的文件和依赖模块图保留各自的 vet 行为。缓存字节永远不能替代测试、绑定源码的前端验证或平台冒烟验收。

发布 job 传输经过冒烟测试的 Linux 镜像、服务端归档和客户端产物，校验源码与 SHA-256 清单后再加载客户端镜像，比对冒烟证据，并运行隔离的三服务网关冒烟测试；它从不在发布阶段重新构建这些字节。

## 参考

- [GitHub 并发控制](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency)说明运行中的取消和待运行替换。发布需要独立的并发组；只在共享分支组中关闭取消，无法防止之后的普通运行影响发布。
- [固定版本的 setup-go 缓存行为](https://github.com/actions/setup-go/blob/b7ad1dad31e06c5925ef5d2fc7ad053ef454303e/src/cache-save.ts)在精确命中时跳过保存。显式的、感知源码的构建 key 使编译缓存可以刷新，且不会混用 `regular` 和 `race` 的写入方。
- [Go 测试与缓存](https://pkg.go.dev/cmd/go#hdr-Test_packages)说明隐式 vet；[Go 构建缓存](https://pkg.go.dev/cmd/go#hdr-Build_and_test_caching)说明 cgo 外部库失效的局限。因此这里仍需不使用缓存的测试执行。
