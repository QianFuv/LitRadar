# 版本发布

本文档说明 LitRadar 的版本规则、发布流水线、发行包运行方式和失败重试，并作为 `README.md` 随服务端发行包分发。示例版本 `0.6.1` 请替换为实际版本。

## 版本规则

根目录 `VERSION` 文件是唯一的版本来源，内容为不带 `v` 前缀的稳定 `MAJOR.MINOR.PATCH` 版本。前端私有包 `apps/client/frontend/package.json` 中的版本不会触发发布。

## 发布一个版本

1. 修改 `VERSION`，例如从 `0.6.0` 改为 `0.6.1`。
2. 运行 `pnpm generate:api` 重新生成前端 API 产物。已提交的 `apps/client/frontend/lib/generated/openapi.json` 在 `info.version` 中记录版本；它与生成结果不一致时，前端质量检查失败。
3. 把两处修改与要发布的内容一起提交，并推送到 `main`。
4. CI 比较整个推送前后的版本，运行后端、前端和客户端质量检查，只有版本递增时才构建发布产物。

普通推送只运行质量检查，不构建发布镜像或归档；测试套件仍可能为验证而编译二进制和前端。版本降级或格式错误会使 CI 失败。单独推送的 tag 不会触发发布，也不需要手动创建 tag 或 Release。

## 发布产物

后端、前端和客户端质量检查通过后，发布 job 从同一源码提交构建服务端和客户端。发布要求全部必需 job 成功，并校验传输的 checksum、源码 SHA、服务端冒烟证据和客户端镜像身份。公开资产：

- `litradar-server_<version>_linux_amd64.tar.gz`
- `litradar-server_<version>_windows_amd64.zip`
- `litradar-client-web_<version>_linux_amd64.tar.gz`（Wails Server Build 二进制）
- `litradar-client_<version>_windows_amd64.zip`（未签名便携版；数据保存在 `litradar-client.exe` 同目录的 `data\`）
- `litradar-client_<version>_android_arm64.apk`（release 签名，仅在配置签名时发布）
- 覆盖全部已发布二进制/包的 `SHA256SUMS`

OCI 镜像为 `ghcr.io/qianfuv/litradar-server` 与 `ghcr.io/qianfuv/litradar-client-web`，各有 `v<version>` 和 `latest` 两个 tag，均为 linux/amd64。可复现部署优先使用版本 tag。Git tag `v<version>` 指向 CI 的精确源码提交；串行的 promotion 步骤把两个 `latest` 都指向已发布的最高稳定版本。服务端打包脚本在解包前拒绝不支持的目标平台。原生客户端只有 Windows x64 和 Android arm64-v8a，没有 iOS、macOS 或 Linux 桌面 job；[Wails 官方把移动端支持标记为实验性](https://wails.io/blog/wails-v3-released)。

Release notes 列出从发布提交可达的、更早的最高已发布稳定版本以来的全部提交。Conventional Commit 前缀按类型分组，无法识别的标题和合并提交放在 Other commits 下。每条记录链接到完整提交，并附完整比较链接。首个稳定版本列出全部历史。草稿重试会重新生成 notes；已发布的 notes 保持不变。

## 工作流

- `CI` 在 `main` 推送和 pull request 上运行。它检查发布规则、Linux/Windows Go 代码、前端和全部客户端目标，并在 `VERSION` 递增时调用 `Release`。
- `CI` 也接受在 `main` 上手动重试当前版本。它运行同样的检查和完整发布流程，没有单独的补发或 promotion 入口。
- `Release` 和可复用检查工作流只供内部调用。
- `Test Diagnostics` 每周或手动运行，只产出参考性的覆盖率产物。

工作流阶段、质量 job 和缓存见 [CI 执行](https://github.com/QianFuv/LitRadar/blob/main/docs/operations/ci.md)。

## 客户端打包

服务端打包与 UI 构建无关。客户端发布 job 安装固定的 Task v3.54.0 和与根 `go.mod` 一致的 Wails CLI，用根 pnpm lockfile 安装前端依赖，并在 `apps/client` 中调用以下固定入口：

| 目标    | 命令                   | Task 输出                                                                       |
| ------- | ---------------------- | ------------------------------------------------------------------------------- |
| Web     | `task package:web`     | `bin/litradar-client-web_<version>_linux_amd64.tar.gz`（linux/amd64，禁用 CGO） |
| Windows | `task package:windows` | `bin/litradar-client_<version>_windows_amd64.zip`                               |
| Android | `task package:android` | `bin/litradar-client_<version>_android_arm64.apk`                               |

Windows 在 Windows x64 runner 上构建，便携版可执行文件未签名。Android 使用 Ubuntu runner、Java 21，并通过 sdkmanager 安装 Android 36/build-tools 36.0.0 和 NDK r28c（`28.2.13676358`）。质量 job 覆盖全部三个 Task 目标；Web host 检查只属于客户端工作流。Android 质量构建可以使用开发签名。

客户端清单绑定精确源码、平台和文件 hash（包括镜像 tar）。发布 job 先校验清单再加载已接受的镜像；加载的镜像必须与冒烟测试的镜像 ID、版本和 revision 一致。缺失或被修改的必需客户端产物会使发布失败。

### Android release 签名

配置全部四个仓库 secret：

- `ANDROID_SIGNING_KEYSTORE`：base64 编码的 release keystore。CI 把它解码为临时文件，并通过 `ANDROID_SIGNING_KEYSTORE` 环境变量传入文件路径。
- `ANDROID_SIGNING_KEYSTORE_PASSWORD`
- `ANDROID_SIGNING_KEY_ALIAS`
- `ANDROID_SIGNING_KEY_PASSWORD`

其余三个 secret 直接映射为同名环境变量，由 `task package:android` 用于 release 签名。CI 用 apksigner 验证 APK，用 keytool 比对其签名证书与配置的 keystore，只清点输出/证据目录，并在始终运行的清理步骤中删除解码后的 keystore。不上传任何签名文件。

任一 secret 缺失时，Android 发布通道记录 `android: omitted`，写入明确的 job summary 警告并上传省略证据。服务端、Web 和 Windows 产物继续发布；不会用调试签名 APK 代替，release notes 会明确说明 Android 被省略。签名配置完整但 key 或密码无效时，发布通道失败。

## 重试发布

在 `CI` 上选择 **Run workflow**，或运行：

```sh
gh workflow run ci.yaml --ref main -f version=0.6.1
```

请求的版本必须与所选 `main` 提交中的 `VERSION` 一致。全部质量套件以及必需的构建和冒烟门禁都会运行。已有 tag 和草稿必须属于同一提交；已发布的资产永不重建或替换。对已发布 Release 的同提交重试只在质量检查后更新 `latest` 并验证 Release。

如果在任何 tag 或草稿存在之前需要修复工作流，推送修复时不要修改 `VERSION`，并在修复后的 `main` 提交上重试；否则在原提交上重试原运行。不要移动已发布的 tag 来修复发布；需要新源码提交时提升版本。

镜像发布与 GitHub 发布是相互独立的服务。瞬时失败可能导致版本镜像已发布而 Release 仍是草稿；重试原工作流会继续发布，并且只在发布完成后更新 `latest`。

## 运行 Linux 服务端归档

Linux 包面向 amd64 上的 Ubuntu 26.04，包含原生 Go 服务端二进制、期刊目录 bundle、Obscura 及其 worker 和第三方声明。Go 服务端二进制不依赖 C 运行时，但随附的 Obscura 二进制和系统 Poppler 工具是动态链接的，因此该发行包不是完全独立的。先安装系统依赖：

```sh
sudo apt-get update
sudo apt-get install ca-certificates libgcc-s1 libstdc++6 poppler-utils poppler-data openssl
sha256sum --check --ignore-missing SHA256SUMS
tar -xzf litradar-server_0.6.1_linux_amd64.tar.gz
cd litradar-server_0.6.1_linux_amd64
mkdir -p secrets
test -e secrets/litradar.key || (umask 077; openssl rand -out secrets/litradar.key 32)
./run.sh --version
./run.sh serve --secret-key-file secrets/litradar.key
```

`run.sh` 以归档目录为工作目录，使用随附的 Obscura，并在缺少 `--project-root` 时把归档目录作为数据根；`pdftotext` 由已安装的 Poppler 包提供。直接运行 `litradar` 时，把 `LITRADAR_OBSCURA_PATH` 设为随附的 Obscura 并在归档目录中运行，或用 `--project-root` 指向可写的数据根。服务端提供 API、文档和健康检查；`/` 返回 404，Web 客户端需在网关后单独部署。期刊目录优先使用系统 bundle，其次使用可执行文件旁的 `assets/meta/`，并保留已自定义的目录文件。

升级时保留部署密钥和现有 `data/`。先停止旧服务并备份数据，再替换可执行文件和资源；不要覆盖数据目录，也不要为已有部署重新生成密钥。admin、index、CFP 和调度命令都可通过 `run.sh` 使用。

## 在 Windows x64 上运行服务端

把服务端 ZIP 解压到可写目录。需要 Windows 10/11 或带 PowerShell 5.1 的 Windows Server；不需要 Go、Node.js、Docker 或开发工具。发行包包含 Obscura render/stealth、Poppler PDF 提取及其原生运行库；检索分词内置于可执行文件，渲染使用 Windows 系统字体。

在 PowerShell 中对照 `SHA256SUMS` 校验 ZIP，再解压：

```powershell
Get-FileHash .\litradar-server_0.6.1_windows_amd64.zip -Algorithm SHA256
Expand-Archive .\litradar-server_0.6.1_windows_amd64.zip -DestinationPath .
Set-Location .\litradar-server_0.6.1_windows_amd64
New-Item -ItemType Directory -Force secrets | Out-Null
if (-not (Test-Path secrets/litradar.key)) {
    $keyBytes = New-Object byte[] 32
    $generator = [Security.Cryptography.RandomNumberGenerator]::Create()
    try { $generator.GetBytes($keyBytes) } finally { $generator.Dispose() }
    [IO.File]::WriteAllBytes((Join-Path $PWD 'secrets/litradar.key'), $keyBytes)
}
powershell -NoProfile -ExecutionPolicy Bypass -File .\run.ps1 --version
powershell -NoProfile -ExecutionPolicy Bypass -File .\run.ps1 serve --secret-key-file secrets/litradar.key
```

执行策略选项只作用于这个启动器进程。启动器选择自身目录和随附的辅助程序，同时保留显式设置的辅助程序路径。全部 CLI 命令都可用。升级时保留 `data/` 和 `secrets/`；不要解压覆盖正在运行的服务。

## 发行包构建边界

服务端可执行文件在 Go 中完成检索分词，不包含 UI；客户端二进制嵌入共享的 Vite 构建。发行归档不包含外部 `web/` 目录或 SQLite 分词库。服务端以禁用 CGO 的方式构建，构建时不需要 C 编译器，运行时不需要 C 运行库。Obscura、Poppler、CA 证书和期刊目录仍有各自的部署要求。Go 清单记录工具链和模块身份。冒烟测试把可执行文件迁移到全新根目录，验证旧 `unicode61` v8 索引的迁移和认证检索。

## 运行 Web 客户端二进制

在 Linux amd64 上下载 `litradar-client-web_<version>_linux_amd64.tar.gz`，对照 `SHA256SUMS` 校验后用 `tar -xzf` 解压，目录中包含可执行文件 `litradar-client-web`。在私有网络中用 `WAILS_SERVER_HOST`/`WAILS_SERVER_PORT` 启动它，并放在与 Compose 相同的 Caddy 路由之后。该二进制只提供 Vite SPA，Go API 服务端仍需单独运行。Windows 和 Android 应用会要求填写服务端 URL 并使用原生设备令牌，见[客户端指南](https://github.com/QianFuv/LitRadar/blob/main/docs/guides/clients.md)。
