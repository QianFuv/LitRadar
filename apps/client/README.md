# LitRadar client hosts

`apps/client` shares the repository root Go module and dependencies with the server. It uses Wails v3.1.0 and hosts the Vite frontend in `frontend/` three ways:

| Target | Build tags | Output |
| --- | --- | --- |
| Web host (linux/amd64, served behind the reverse proxy) | `server,production` | `bin/litradar-client-web` |
| Windows amd64 desktop app | `production` | `bin/litradar-client.exe`, NSIS installer `bin/litradar-client-windows-amd64-setup.exe` |
| Android arm64-v8a app | `android,production` | `bin/litradar-client-android-arm64.apk` |

macOS, Linux desktop, and iOS are not built.

## Tasks

Run from `apps/client` with `wails3 task <name>` (or `task <name>`). Every build task first builds
the frontend from the repository root workspace in the right Vite mode: `web` for the web host,
`native` for the desktop and Android apps. The native mode aliases `lib/platform/active` to
`lib/platform/native.tsx`, so `@wailsio/runtime` never enters the web bundle.

| Task | Result |
| --- | --- |
| `build:server` | `bin/litradar-client-web` (`CGO_ENABLED=0`, linux/amd64) |
| `build:docker` | distroless image from `build/docker/Dockerfile.server` |
| `build:windows` | `bin/litradar-client.exe` (no CGO; WebView2) |
| `package:windows` | `bin/litradar-client-windows-amd64-setup.exe` (needs `makensis`) |
| `package:android` | release APK `bin/litradar-client-android-arm64.apk` |
| `build:android:debug` | debuggable APK `bin/litradar-client-android-arm64-debug.apk` with an inspectable WebView |
| `build:windows:smoke` | `bin/litradar-client-smoke.exe` for the desktop smoke test |
| `test` | `go vet` and `go test` for `internal/...` |

`WAILS3` overrides the Wails CLI used for `generate syso` and the WebView2 bootstrapper; it
defaults to `go run github.com/wailsapp/wails/v3/cmd/wails3@v3.1.0`.

## Requirements

- Go 1.26+, Node 24, pnpm 12 (the workspace root `pnpm-lock.yaml` pins `@wailsio/runtime` 3.1.0).
- Windows installer: NSIS 3 (`makensis` on `PATH`). The installer bundles the WebView2
  bootstrapper, which `package:windows` downloads with the Wails CLI.
- Android:
  - `ANDROID_HOME` (or `ANDROID_SDK_ROOT`): SDK with platform 36 and build-tools.
  - `ANDROID_NDK_HOME`: optional; defaults to the newest NDK under `$ANDROID_HOME/ndk`
    (built and verified with r28c, `28.2.13676358`).
  - `JAVA_HOME`: JDK 17 or newer for Gradle 9.2 and AGP 9.0 (verified with JDK 21).
  - Gradle downloads its distribution and Maven dependencies on the first build.
  - Windows and Linux hosts are supported; the Go library is compiled with the NDK clang of
    the host (`*-clang.cmd` on Windows).

### Android release signing

`package:android` signs with the keystore given by these variables, and with the Android debug
keystore when `ANDROID_SIGNING_KEYSTORE` is unset (testing only):

| Variable | Meaning |
| --- | --- |
| `ANDROID_SIGNING_KEYSTORE` | keystore path |
| `ANDROID_SIGNING_KEYSTORE_PASSWORD` | keystore password |
| `ANDROID_SIGNING_KEY_ALIAS` | key alias |
| `ANDROID_SIGNING_KEY_PASSWORD` | key password |

## Native host design

- The Go `internal/nativehost.Bridge` service owns all server traffic. The frontend calls it
  through `Call.ByName`; the bridge restricts requests to the configured origin (`https://` for
  any host, `http://` only for loopback) and to `/api/` plus `/health/ready`, forwards only an
  allowlist of headers, never follows redirects, and attaches `Authorization: Bearer` from OS
  secure storage. JavaScript never receives the token. Request timeouts and retries stay in
  `frontend/lib/api/client.tsx`; cancelling a request cancels the Go call context.
- Tokens are stored per server origin under `LitRadar/token/<origin>`: Windows Credential
  Manager on desktop, Wails `SecureSet` (EncryptedSharedPreferences) on Android. The selected
  origin is not secret and lives in `client.json` in the user config directory
  (`%AppData%\LitRadar` on Windows, the app files directory on Android).
- Native clients sign in only with a password or single sign-on. Password sign-in uses
  `POST /api/auth/native/login`, which issues a `device:<name>` personal access token.
- Single sign-on (`Bridge.LoginWithOidc`) runs in the system browser. The bridge generates the
  PKCE verifier, S256 challenge, and state, opens
  `/api/auth/oidc/native/login?redirect_uri=…&code_challenge=…&state=…`, waits up to five
  minutes for the callback with the matching state, and exchanges the one-time code through
  `POST /api/auth/oidc/native/exchange`; the code, verifier, and token never reach JavaScript.
  Windows receives the callback on a one-shot listener at `http://127.0.0.1:<port>/oidc/callback`
  (GET, matching state, first request only) and then asks Windows to bring the window forward.
  Android receives `litradar://oidc/callback` through the activity's intent filter
  (`LitRadarOidc.java` hands the link to Go over JNI). When the activity or process was recreated
  while the browser was open, the callback is reported as an interrupted sign-in on the login
  page. Linking and unlinking single sign-on stay web-only.
- Logout revokes the token on the server and deletes it locally even when the server is
  unreachable. Switching servers in the settings revokes and deletes the token of the current
  server, clears cached queries and the local user cache, and returns to server setup.
- Windows downloads use the system save dialog. Android downloads go to the public Downloads
  collection through `LitRadarDownloads.java` (MediaStore, no storage permission on API 29+,
  which is why `minSdk` is 29). External links open in the system browser.
- The Android Java package stays `com.wails.app` because the Wails Go runtime exports its JNI
  entry points for that package; the installed application id is `com.qianfuv.litradar`.

## Smoke tests

Both drivers start a disposable Go server seeded with the full-stack fixture. Build the server
binaries first with `node apps/server/scripts/build-go.mjs` from the repository root, then run from
`apps/client/frontend`:

- `node tests/native-smoke/windows-smoke.mjs` after `task build:windows:smoke`. The smoke build
  is the release build plus a WebView2 remote debugging port read from
  `LITRADAR_CLIENT_REMOTE_DEBUGGING_PORT` (Wails clears `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`,
  and release builds contain no debugging hook). It drives setup, login, search, favorite,
  export through the real save dialog, restart, logout, and server switch, writing evidence to
  `test-results/native-windows`. It refuses to run when `%AppData%\LitRadar\client.json` exists.
- `node tests/native-smoke/android-smoke.mjs` with a debuggable APK installed on a running
  device or emulator. The server port is mapped into the device with `adb reverse`, so the app
  uses a loopback `http://127.0.0.1:<port>` origin. Evidence goes to
  `test-results/native-android`.
- `node tests/native-smoke/windows-oidc-smoke.mjs` and `node tests/native-smoke/android-oidc-smoke.mjs`
  cover single sign-on against `fake-oidc-provider.mjs`, with the fixture server in development
  mode so the provider may use plain HTTP. On Windows the smoke build writes the URLs it would
  open to `LITRADAR_CLIENT_SMOKE_BROWSER_URL_FILE`, and the driver opens them in its own
  Chromium; the journey covers cancel, deny, success, restart, and logout. On Android the real
  Chrome on the device is the browser (start it once with first-run screens disabled); the
  journey covers a sign-in whose app process is killed while Chrome is open, then a normal
  sign-in and restart.

Playwright's `connectOverCDP` emulates `prefers-color-scheme: light` on attached pages;
`attachToAppPage` clears that emulation so screenshots follow the system light or dark mode.

The arm64 APK does not run on x86_64 emulator images with ARM translation: the Go arm64 runtime
reads `MIDR_EL1`, which the translator does not implement (`SIGILL`). Use an arm64 device, or an
x86_64 build of the library for emulator-only checks.
