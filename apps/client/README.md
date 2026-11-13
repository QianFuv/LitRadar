# LitRadar 客户端宿主

`apps/client` 与服务端共用仓库根 Go 模块及其依赖，使用 Wails v3.1.0，以三种方式承载 `frontend/` 中的 Vite 前端：

| 目标                                      | 构建 tags            | 产物                                                                                    |
| ----------------------------------------- | -------------------- | --------------------------------------------------------------------------------------- |
| Web 主机（linux/amd64，位于反向代理之后） | `server,production`  | `bin/litradar-client-web`，归档 `bin/litradar-client-web_<version>_linux_amd64.tar.gz`  |
| Windows amd64 桌面应用                    | `production`         | `bin/litradar-client.exe`，便携版 zip `bin/litradar-client_<version>_windows_amd64.zip` |
| Android arm64-v8a 应用                    | `android,production` | `bin/litradar-client_<version>_android_arm64.apk`                                       |

不构建 macOS、Linux 桌面和 iOS 客户端。

## 任务

在 `apps/client` 中以 `wails3 task <name>`（或 `task <name>`）运行。每个构建任务都会先在仓库根 workspace 中按对应 Vite 模式构建前端：Web 主机使用默认模式，桌面与 Android 应用使用 `native` 模式。`native` 模式把 `lib/platform/active` 别名到 `lib/platform/native.tsx`，因此 `@wailsio/runtime` 不会进入 Web 包。

| 任务                                       | 结果                                                                                |
| ------------------------------------------ | ----------------------------------------------------------------------------------- |
| `build:frontend` / `build:frontend:native` | 在根 workspace 构建 `frontend/dist`（默认模式 / `native` 模式）                     |
| `build:server`                             | `bin/litradar-client-web`（`CGO_ENABLED=0`，linux/amd64）                           |
| `run:server`                               | 以当前平台运行 Web 主机，端口取 `WAILS_SERVER_PORT`（默认 8080）                    |
| `package:web`                              | 归档 `bin/litradar-client-web_<version>_linux_amd64.tar.gz`                         |
| `build:docker`                             | 基于 `build/docker/Dockerfile.server` 的 distroless 镜像（`TAG=name:tag`）          |
| `build:windows`                            | `bin/litradar-client.exe`（不使用 CGO；WebView2）                                   |
| `package:windows`                          | 便携版 zip `bin/litradar-client_<version>_windows_amd64.zip`（需在 Windows 上运行） |
| `package:android`                          | 发布 APK `bin/litradar-client_<version>_android_arm64.apk`                          |
| `build:android:debug`                      | 可调试 APK `bin/litradar-client_<version>_android_arm64_debug.apk`，WebView 可检查  |
| `build:windows:smoke`                      | 桌面冒烟测试使用的 `bin/litradar-client-smoke.exe`                                  |
| `test`                                     | 对 `internal/...` 运行 `go vet` 与 `go test`                                        |

`WAILS3` 覆盖 `generate syso` 使用的 Wails CLI，默认值为 `go run github.com/wailsapp/wails/v3/cmd/wails3@v3.1.0`。

客户端版本只从仓库根 `VERSION` 文件派生，客户端构建文件中不写版本号；发布时的完整改动步骤见[版本发布](../../docs/operations/releases.md)。各构建的派生方式：

- Android：`build/android/app/build.gradle` 把 `VERSION` 读入 `versionName`，并设置 `versionCode` 为 `major * 10000 + minor * 100 + patch`；`VERSION` 格式错误时构建失败。
- Windows 可执行文件：`scripts/client-version.mjs windows-resources` 把 `build/windows/info.json`（不含版本字段）和 `build/windows/wails.exe.manifest`（其中 `{{FILE_VERSION}}` 占位符替换为 `MAJOR.MINOR.PATCH.0`）渲染到 `bin/windows-resources`，供 `generate syso` 使用。

所有包都按 `litradar-<component>_<version>_<os>_<arch>.<ext>` 命名，命名只在 `scripts/package-client.mjs` 中定义；CI 只暂存这些文件。Web 主机归档包含一个 `litradar-client-web_<version>_linux_amd64` 目录，内含可执行文件与 `LICENSE`。

Windows 便携版 zip 包含一个 `litradar-client_<version>_windows_amd64` 目录，内含 `litradar-client.exe` 与 `LICENSE`。应用把宿主配置和 WebView2 配置文件保存在可执行文件同目录的 `data` 文件夹中，令牌保存在 Windows 凭据管理器。运行需要 Windows 10 和 11 自带的 WebView2 运行时。

## 环境要求

- Go、Node.js 与 pnpm 的固定版本见[开发指南](../../docs/guides/development.md#工具链)；仓库根 `pnpm-lock.yaml` 锁定 `@wailsio/runtime` 3.1.0。
- Task v3（CI 固定 v3.54.0），或通过 `wails3 task` 运行。
- Android：
  - `ANDROID_HOME`（或 `ANDROID_SDK_ROOT`）：包含 platform 36 与 build-tools 的 SDK。
  - `ANDROID_NDK_HOME`：可选；默认使用 `$ANDROID_HOME/ndk` 下最新的 NDK（已用 r28c `28.2.13676358` 构建和验证）。
  - `JAVA_HOME`：Gradle 9.2 与 AGP 9.0 需要 JDK 17 或更高版本（已用 JDK 21 验证）。
  - Gradle 首次构建时下载其发行版和 Maven 依赖。
  - 支持 Windows 和 Linux 主机；Go 库使用主机对应的 NDK clang 编译（Windows 上为 `*-clang.cmd`）。

### Android 发布签名

`package:android` 使用以下变量指定的 keystore 签名；未设置 `ANDROID_SIGNING_KEYSTORE` 时改用 Android debug keystore（仅用于测试）。CI 中对应 secrets 的配置与校验见[版本发布](../../docs/operations/releases.md)。

| 变量                                | 含义          |
| ----------------------------------- | ------------- |
| `ANDROID_SIGNING_KEYSTORE`          | keystore 路径 |
| `ANDROID_SIGNING_KEYSTORE_PASSWORD` | keystore 密码 |
| `ANDROID_SIGNING_KEY_ALIAS`         | key 别名      |
| `ANDROID_SIGNING_KEY_PASSWORD`      | key 密码      |

## 原生宿主设计

- Go 的 `internal/nativehost.Bridge` 服务负责全部服务端通信。前端通过 `Call.ByName` 调用它；桥只允许访问已配置的 origin（任意主机的 `https://`，或仅限 loopback 的 `http://`）下的 `/api/` 与 `/health/ready`，只转发白名单内的请求头，从不跟随重定向，并从系统安全存储附加 `Authorization: Bearer`。JavaScript 永远拿不到令牌。请求超时与重试仍由 `frontend/lib/api/client.tsx` 负责；取消请求会取消 Go 调用的 context。
- 令牌按服务端 origin 保存在 `LitRadar/token/<origin>` 下：桌面端为 Windows 凭据管理器，Android 为 Wails `SecureSet`（EncryptedSharedPreferences）。所选 origin 不是秘密，保存在 `client.json` 中：Windows 上位于 `litradar-client.exe` 同目录的便携 `data` 文件夹（也保存 WebView2 配置文件），Android 上位于应用文件目录。
- 原生客户端只支持密码登录和单点登录。密码登录使用 `POST /api/auth/native/login`，签发 `device:<name>` 个人访问令牌。
- 单点登录（`Bridge.LoginWithOidc`）在系统浏览器中进行。桥生成 PKCE verifier、S256 challenge 和 state，打开 `/api/auth/oidc/native/login?redirect_uri=…&code_challenge=…&state=…`，最多等待五分钟接收 state 匹配的回调，再通过 `POST /api/auth/oidc/native/exchange` 兑换一次性授权码；授权码、verifier 和令牌都不会进入 JavaScript。Windows 在一次性监听器 `http://127.0.0.1:<port>/oidc/callback` 上接收回调（GET、state 匹配、只处理第一个请求），随后请求 Windows 把窗口置前。Android 通过 activity 的 intent filter 接收 `litradar://oidc/callback`（`LitRadarOidc.java` 经 JNI 把链接交给 Go）。浏览器打开期间 activity 或进程被重建时，登录页把该回调报告为中断的登录。绑定和解绑单点登录只在 Web 端提供。
- 退出登录会在服务端撤销令牌，并在服务端不可达时同样删除本地令牌。在设置中切换服务端会撤销并删除当前服务端的令牌，清空查询缓存与本地用户缓存，并回到服务端设置页。
- Windows 下载使用系统保存对话框。Android 下载通过 `LitRadarDownloads.java` 写入公共 Downloads 集合（MediaStore；API 29 起不需要存储权限，因此 `minSdk` 为 29）。外部链接在系统浏览器中打开。
- Android Java 包名保持 `com.wails.app`，因为 Wails Go 运行时为该包导出 JNI 入口；安装后的 application id 为 `com.qianfuv.litradar`。

## 冒烟测试

两个驱动都会启动一个预置 full-stack fixture 的一次性 Go 服务端。先在仓库根目录运行 `node apps/server/scripts/build-go.mjs` 构建服务端二进制，再在 `apps/client/frontend` 中运行：

- `node tests/native-smoke/windows-smoke.mjs`：先执行 `task build:windows:smoke`。冒烟构建是在发布构建基础上增加一个 WebView2 远程调试端口，端口从 `LITRADAR_CLIENT_REMOTE_DEBUGGING_PORT` 读取（Wails 会清除 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`，发布构建不含调试钩子）。它驱动服务端设置、登录、检索、收藏、经真实保存对话框导出、重启、退出和切换服务端，证据写入 `test-results/native-windows`。存在 `bin\data\client.json` 时拒绝运行。
- `node tests/native-smoke/android-smoke.mjs`：需要在运行中的设备或模拟器上安装可调试 APK。服务端端口通过 `adb reverse` 映射到设备，因此应用使用 loopback `http://127.0.0.1:<port>` origin。证据写入 `test-results/native-android`。
- `node tests/native-smoke/windows-oidc-smoke.mjs` 与 `node tests/native-smoke/android-oidc-smoke.mjs`：针对 `fake-oidc-provider.mjs` 覆盖单点登录，fixture 服务端运行在开发模式，因此 provider 可以使用明文 HTTP。Windows 上冒烟构建把本应打开的 URL 写入 `LITRADAR_CLIENT_SMOKE_BROWSER_URL_FILE`，由驱动在自己的 Chromium 中打开；旅程覆盖取消、拒绝、成功、重启和退出。Android 上浏览器是设备中的真实 Chrome（先以关闭首次运行页面的方式启动一次）；旅程覆盖 Chrome 打开期间应用进程被杀死的登录，然后是一次正常登录和重启。

Playwright 的 `connectOverCDP` 会在附加的页面上模拟 `prefers-color-scheme: light`；`attachToAppPage` 清除该模拟，使截图跟随系统的浅色或深色模式。

arm64 APK 不能在带 ARM 转译的 x86_64 模拟器镜像上运行：Go arm64 运行时会读取 `MIDR_EL1`，而转译器未实现该寄存器（`SIGILL`）。请使用 arm64 设备，或为仅限模拟器的检查构建 x86_64 版本的库。
