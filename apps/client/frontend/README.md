# LitRadar 前端包

`apps/client/frontend/` 是 LitRadar 的 Vite + React 单页客户端源码，负责登录、检索、收藏、每周更新、征稿追踪、聚合设置中心和管理后台。发布构建输出静态 `dist/`，由 `apps/client` 的 Wails v3 Server Build 嵌入并提供（SPA 回退与安全响应头）；Go API 服务不再托管网页，生产环境由外部反向代理把 `/api*`、`/mcp*`、`/docs*`、`/openapi.json`、`/openapi.yaml`、`/health/*` 转发到 Go 服务、其余路径转发到 Web 客户端。本页只说明前端包的开发边界：

- 系统进程与数据流见[系统架构](../../../docs/architecture/README.md)。
- REST 契约与认证方式见[API 参考](../../../docs/reference/api.md)。
- 后端与前端的无环境覆盖配置边界见[运行配置](../../../docs/reference/configuration.md)。
- UI token 与组件约定见[设计系统](../../../docs/reference/design-system.md)。
- 服务端和浏览器日志契约见[日志运维](../../../docs/operations/logging.md)。
- 五层测试模型、功能所有权和报告策略见[测试系统](../../../docs/guides/testing.md)。

## 工具链

CI 与前端构建阶段使用：

| 工具              | 版本                                                                                      |
| ----------------- | ----------------------------------------------------------------------------------------- |
| Node.js           | 26.11.1                                                                                   |
| pnpm              | 12.16.1                                                                                   |
| Vite+（`vp`）     | 1.5.x；内含 Vite 8、Vitest 5、Oxlint、Oxfmt，版本随 Vite+ 锁定                            |
| TanStack Router   | 1.x（代码式路由树，search 用 Zod 4 校验）                                                 |
| React / React DOM | 19.2.3                                                                                    |
| React Compiler    | `babel-plugin-react-compiler` 1.x，经 `@rolldown/plugin-babel` 在 `vite.config.ts` 中启用 |
| Motion for React  | 14.x                                                                                      |
| TypeScript        | 7.x                                                                                       |
| StyleX            | 0.19.x（`@stylexjs/unplugin` 编译为原子 CSS，经 Lightning CSS 处理）                      |
| Go                | 1.27.3；生成 OpenAPI、启动后端和 full-stack e2e 时需要，不需要 C 编译器                   |

依赖由仓库根目录的 pnpm workspace `pnpm-lock.yaml` 锁定。前端状态与 UI 的主要库包括 TanStack Router、TanStack Query、TanStack Pacer（防抖）、Zod、next-themes、Base UI（经 Animate UI primitives 封装）、StyleX、Motion 和 lucide-react；元素可见性用原生 IntersectionObserver（`lib/use-in-view.tsx`）。根目录 `.gitattributes` 让 `apps/client/frontend/**` 在所有平台以 LF 换行签出，与 Oxfmt 的格式检查一致。样式写在各组件文件末尾的 `stylex.create`，颜色与阴影引用 `lib/theme.stylex.tsx` 的 token；接受样式覆盖的组件使用 `xstyle` 属性（类型为 `stylex.StyleXStyles`），不再接受 `className`。Motion 只通过 `components/ui/motion.tsx` 的 `LazyMotion` / `domAnimation` 封装进入业务代码；Base UI 浮层的进出场由 Animate UI primitives 内置的 Motion 过渡驱动。

## 本地运行

准备好前端依赖和仓库部署密钥后，在仓库根目录运行 `pnpm dev`（即 `node scripts/dev.mjs`），即可一起启动 Vite 与 Go 服务；Go 服务的 `--project-root` 固定为仓库根目录。按 `Ctrl+C` 一起关闭；启动失败或任一进程意外退出也会清理另一进程。无需先运行 `pnpm build`，也无需建立静态资源目录连接。完整准备步骤和隔离数据目录参数见[开发指南](../../../docs/guides/development.md)。

需要分开启动时，使用以下命令。

在仓库根目录启动只监听 loopback 8001 的统一 Go 应用（工具链见[开发指南](../../../docs/guides/development.md#工具链)）；HTTP 和内嵌调度共享该进程：

```bash
go run ./apps/server serve \
  --development \
  --host 127.0.0.1 \
  --port 8001 \
  --secret-key-file secrets/litradar.key
```

再在仓库根目录安装 workspace 依赖，并在本目录运行 Vite：

```bash
npm install --global pnpm@12.16.1
pnpm install --frozen-lockfile
pnpm --filter litradar-frontend dev
```

默认浏览器入口统一为 `http://localhost:8000`。Vite 开发服务器保留 HMR，并把 `/api`、`/mcp`（含 SSE）、`/docs`、`/openapi.json`、`/openapi.yaml` 和 `/health` 代理到内部 Go 地址 `http://127.0.0.1:8001`（可用 `LITRADAR_BACKEND_URL` 覆盖）；`pnpm preview` 使用同一代理。浏览器不需要访问第二个端口。

- Web：`http://localhost:8000/`
- REST API：`http://localhost:8000/api`
- API 文档：`http://localhost:8000/docs`（huma 内置文档页，页面脚本从公共 CDN 加载）
- OpenAPI JSON：`http://localhost:8000/openapi.json`
- MCP：`http://localhost:8000/mcp`

发布构建执行 `pnpm build`（`vp build`）并写入 `dist/`；类型检查由 `pnpm check` 负责。兼容基线为去年 1 月 1 日以来发布的全部浏览器版本：`vite.config.ts` 按当前年份生成 browserslist 查询 `since <去年>-01-01`，并由它派生 `build.target` 与 `cssTarget`（2026 年为 Chromium 132、Firefox 134、Safari/iOS 18.3 起）。`apps/client` 中的 `wails3 task build:server` 与 `build:docker` 把它嵌入静态 linux/amd64 Web 主机和 distroless 镜像。

## 前端网络配置

前端没有应用专用环境配置，也不把 API 根地址嵌入构建产物：

- 请求统一经过 `lib/platform/` 的 `ClientPlatform`；Web 实现从 `window.location.origin` 生成同源 API URL 并携带会话 Cookie。
- `lib/api/client.tsx` 为每次请求设置 60 秒超时，传递调用方 `AbortSignal`，并对幂等 GET 在网络失败或不超过 5 秒的 `Retry-After`（429/503）时最多重试两次。
- 非 2xx 响应体按 RFC 9457 `application/problem+json` 用 Zod 解析为 `ApiError`：`errors[]` 中带 `body`/`path`/`query`/`header`/`cookie` 校验位置的条目（如 422）拼成"请求参数无效：字段（原因）"的中文提示，最多列 3 条；其余情况以 `detail` 作为显示文本（缺失时使用调用方的回退文案），`type`、`title`、`instance` 与 `errors[]` 原样保留，`findErrorDetail(location)` 读取 `errors[]` 中某个 `location` 的 `message`（例如 CNKI 工作流的 `code` 与 `phase`）。只有带 `Retry-After` 的 429/503 响应才标记为 `retryable`；格式异常的成员按缺失处理。
- `pnpm dev` / `pnpm preview` 只使用 `vite.config.ts` 中的开发代理。
- 生产环境由外部反向代理让网页与 API 共用一个 Origin。
- 浏览器跨源直连和构建时 API 地址覆盖不再受支持；需要不同公网入口时，应在同一 Origin 前部署反向代理。

## 路由

| 路由                    | 访问边界 | 页面职责                                      |
| ----------------------- | -------- | --------------------------------------------- |
| `/login`                | 公开     | 登录、注册和邀请码状态                        |
| `/`                     | 已登录   | 数据库/领域/期刊/日期筛选、FTS 搜索与文章列表 |
| `/?view=favorites`      | 已登录   | 文件夹、收藏、批量操作与引文导出              |
| `/?view=weekly-updates` | 已登录   | 按数据库和期刊浏览本周变化                    |
| `/?view=cfp-tracking`   | 已登录   | 按维护期刊浏览征稿原文、状态和截止日期        |
| `?settings=<section>`   | 已登录   | 在当前受保护工作区上打开聚合设置中心          |
| `?admin=<section>`      | 管理员   | 在当前受保护工作区上打开分栏管理面板          |

路由树位于 `src/router.tsx`（TanStack Router 代码式路由，不生成路由树文件）：根路由提供错误页与 404，`/login` 是公开页，无路径的工作区布局（route id `/_workspace`）承载受保护的首页。除 `/login` 外，页面都在 `src/routes/require-auth.tsx` 守卫之下，它通过 `AuthProvider` 恢复当前用户，并在认证状态变化时把未登录访问一次性重定向到 `/login?next=...`。管理面板只由全局 Dialog 提供；账号菜单入口与手工添加的 `admin` query 都检查 `is_admin`，普通用户的管理参数会被移除且不会挂载管理数据组件。

根路由通过 `view` 参数切换检索、收藏、每周更新和征稿工作区。省略 `view` 或传入未知值时显示检索；四个固定入口分别使用 `/`、`/?view=favorites`、`/?view=weekly-updates` 和 `/?view=cfp-tracking`，因此可直接访问、刷新并使用浏览器前进/后退。切换固定入口会清除上一工作区的私有参数；收藏继续使用 `folder`，周报继续使用 `db`、`journal` 和 `weekly_q`；征稿使用独立的 `cfp_db`、`cfp_journal`，不覆盖文章检索的数据库偏好。`/favorites` 与 `/weekly-updates` 已直接删除，访问时返回普通 404，不提供重定向或兼容页面。

设置中心由受保护布局全局挂载，合法分类为 `general`、`tracking`、`notifications`、`data-sources`、`account` 和 `tokens`。管理面板同样由该布局挂载，合法分类为 `overview`、`users`、`invite-codes`、`runtime-settings`、`scheduled-tasks` 和 `announcements`。打开、切换与关闭只修改对应的 `settings` 或 `admin` 参数并保留其他工作区状态；两个中心互斥，未知分类会被移除。`/admin`、`/settings` 和 `/tracking` 都不是页面路由，访问时返回普通 404，不提供重定向或兼容页面。

URL search 的契约集中在 `lib/search-params.tsx`：工作区与登录页各有一个 Zod schema，非法值按缺失处理，未知参数原样保留。查询串保持普通的 `key=value` 形式，列表参数（`area`、`journal_id` 与四个评级）以逗号连接、元素内的逗号转义为 `%2C`，与迁移前的链接兼容。组件用 `useSearch({ from: WORKSPACE_ROUTE_ID })` 读取、用 `useUpdateWorkspaceSearch()` 写入；写入替换当前历史记录且不重置滚动位置，空字符串和空列表不会出现在 URL 中。

`index.html` 提供默认标题与描述；四个查询参数工作区和管理 Dialog 共用它，登录页与 404 页通过 React 19 `<title>`/`<meta>` 覆盖。未知路由使用自定义 404 页面；路由错误边界（`src/error-boundary.tsx`）提供重试和首页入口并在导航后复位，Router 或 Providers 失败时由不依赖 Providers 的独立全局错误画面兜底。首屏主题由 `public/theme-init.js` 在渲染前应用，避免闪烁。

四个工作区复用应用框架（桌面导航轨、手机底栏）与两栏阅读工作区（列表栏 + 宽屏阅读台，窄屏改为侧滑详情面板）。设置中心与管理面板复用全屏 Base UI Dialog 外壳；文献追踪和通知分类共享未保存草稿，关闭或离开该分类组前需要确认。字体、配色、尺寸、动效和焦点管理统一见[设计系统](../../../docs/reference/design-system.md)，本页不重复维护视觉参数。

征稿工作区只读取后端认证 API，原文、状态和分页由服务端提供。采集和刷新不在浏览器中执行，见[征稿追踪架构](../../../docs/architecture/cfp-tracking.md)。

## 目录职责

```text
apps/client/frontend/
├── index.html             静态元数据、主题初始化脚本、skip link 与挂载点
├── src/
│   ├── main.tsx           浏览器入口
│   ├── app.tsx            全局错误边界、服务端选择、Providers 与 RouterProvider
│   ├── router.tsx         路由树与 router 工厂
│   ├── devtools.tsx       仅开发环境懒加载的 TanStack Devtools（Router 与 Query 面板）
│   ├── routes/            首页工作区、登录、404 与认证守卫
│   ├── globals.css        字体、元素重置、Animate UI 所需工具类、滚动条与 skip link
│   ├── error-boundary.tsx 可复位的错误边界
│   ├── route-error.tsx    常规路由错误画面与重试入口
│   ├── global-error.tsx   Router 或 Providers 失败时的独立错误画面
│   └── providers.tsx      Theme、Motion、React Query 与认证上下文
├── assets/                 本地字体 CSS 与 Unicode-range WOFF2 分片
├── components/
│   ├── admin/             全局管理面板 Dialog 与管理功能卡片
│   ├── cfp/               征稿工作区、期刊分组与原文展示
│   ├── favorites/         收藏页视图与 view model
│   ├── feature/           阅读工作区、检索与筛选胶囊、文章详情和全局用户菜单
│   ├── shell/             应用框架：导航轨、手机底栏与账号头像
│   ├── settings/          聚合设置中心、分类内容与设置组件
│   ├── tracking/          追踪设置内容与共享 view model
│   ├── weekly/            每周更新工作区视图与查询编排
│   └── ui/                Base UI / Animate UI + StyleX 基础组件
├── lib/
│   ├── api/               按 auth/index/favorites/tracking/admin 拆分的 facade 与共享 transport
│   ├── platform/          ClientPlatform 接口、Web 实现与 Wails 原生实现（`--mode native` 构建）
│   ├── generated/         OpenAPI JSON 与生成的 TypeScript schema
│   ├── api.tsx            API facade 的公共导出
│   ├── api-contract.tsx   安全敏感响应的运行时校验
│   ├── auth-context.tsx   Cookie 会话恢复与认证操作
│   ├── browser-storage.tsx Web Storage helper 与 `parseStoredJson`
│   ├── search-params.tsx  工作区与登录页 URL search 的 Zod schema
│   ├── theme.stylex.tsx   StyleX 设计 token
│   └── client-logger.tsx  浏览器本地、隐私受限的错误事件
└── tests/
    ├── *.test.tsx         Vitest/jsdom 与显式 MSW 场景
    ├── browser-components/ 选择性的 Vitest Chromium 原生语义与动效生命周期
    ├── mocks/             按领域组织的 typed scenario handlers
    └── e2e/
        ├── local-fixtures.spec.tsx  使用显式 API 场景的 UI 冒烟测试
        └── full-stack/              真实 Go/SQLite 关键旅程
```

新增业务逻辑时优先放入对应 feature 目录；可复用的视觉原语放入 `components/ui/`。新应用 TypeScript 源文件默认使用 `.tsx`，即使不含 JSX；既有扩展名、声明文件和工具要求的配置文件按仓库约定保留。

## 客户端状态

| 状态                   | 所有者                                                                               |
| ---------------------- | ------------------------------------------------------------------------------------ |
| 后端查询与 mutation    | TanStack Query                                                                       |
| 工作区、搜索和筛选选择 | TanStack Router URL search（`lib/search-params.tsx` 的 Zod schema）                  |
| 登录用户               | `AuthProvider` + `GET /api/auth/me`；非秘密用户缓存 `localStorage: litradar:v1:user` |
| 退出撤销未确认提示     | `localStorage: litradar:v1:logout_revocation_unconfirmed`                            |
| 当前数据库             | `localStorage: litradar:v1:selected_database`                                        |
| 搜索历史               | `localStorage: litradar:v1:search_history`                                           |
| 主题                   | next-themes 的 `class` 属性与系统偏好                                                |

浏览器 API 请求默认 `credentials: include`，登录令牌只存在后端设置的 `litradar_session` HttpOnly Cookie 中。设置中心创建的 Bearer 访问令牌用于外部客户端，不作为前端登录态存入 Web Storage。

旧版搜索历史键 `search_history` 和数据库选择键 `selected_database` 会迁移到对应的新命名空间并删除；其他升级前的浏览器键不会被读取或复制。升级后用户需要重新登录。

Web Storage helper 会容忍 SSR、隐私模式和 quota 错误；调用方不应假定写入必然成功。读取的 JSON 值（搜索历史、退出撤销提示、检索构建器与关系图会话快照）经 `parseStoredJson` 用 Zod 校验，不符合的值按缺失处理。

站点图标、导航轨与登录页品牌图像使用 `public/litradar-logo.png` 本地静态资源，不依赖第三方图片域名。

## 浏览器错误日志

route/global Error Boundary、`window.error` 和 `unhandledrejection` 共用 `client-logger.tsx`。它只向当前浏览器的 `console.error` 写一条包含安全分类摘要和冻结对象的日志；摘要为 `[client.error] <source>: <error_kind>`，便于开发错误面板显示。对象字段为 `timestamp`、`level=error`、`event=client.error`、`component=browser`、`source`、不含 query 的 route pathname、`error_kind`，以及可选安全 `digest`/`request_id`。

记录器不枚举原 Error，不写 message、stack、promise reason、请求/响应 body、token、query 或 Web Storage，也不发起 fetch/beacon。对象身份用于去重，Providers 在 React Strict Mode 下确定性安装和移除全局 listener。非 2xx API 响应的 `X-Request-Id` 保存在 `ApiError.requestId`；错误边界可以把它作为安全关联 ID 输出，运维人员再按同一 ID 查询服务端日志。完整示例与事故流程见[日志运维](../../../docs/operations/logging.md)。

## API 契约

REST schema 由 Go 类型生成：`apps/server/internal/api/` 用 huma 注册各操作的请求与响应类型，`apps/server/internal/openapi/` 负责文档配置、操作 ID 与共享 schema 注册，运行时据此输出 OpenAPI 3.1。前端生成物：

- `lib/generated/openapi.json`
- `lib/generated/api-schema.tsx`

后端路由、请求/响应类型或 OpenAPI 配置变化后运行：

```bash
pnpm generate:api
```

CI 使用：

```bash
pnpm generate:api:check
```

生成命令会运行 Go `litradar openapi` 子命令、生成 TypeScript 类型并用 Oxfmt 格式化产物。openapi-typescript 依赖 TypeScript 5/6 的编译器 JS API，而 TypeScript 7 包不再提供该 API，因此脚本把它的 `typescript` 导入解析到 `@typescript/typescript6`。不要手工修改 `lib/generated/`。

`lib/api/` 提供面向页面的请求 facade；`lib/api-contract.tsx` 对认证、运行设置元数据、Provider 能力目录、计划任务和手动推送等控制面响应再做运行时校验。管理页按后端返回的 group、control、apply mode 和 allowed values 渲染全部设置，并按 capability 过滤 Provider 选项。普通页面不应绕过共享 transport 自行复制 Cookie、Bearer、数据库选择或错误解析逻辑。

## 质量检查

以下命令在 `apps/client/frontend/` 中运行。日常修改按[测试系统](../../../docs/guides/testing.md)选择适用检查；涉及完整前端交付时使用下列检查序列，任一步失败都应先定位原因：

```bash
pnpm generate:api:check
pnpm check
pnpm knip
pnpm test:unit
pnpm exec playwright install --with-deps chromium
pnpm test:browser-components
pnpm test:e2e:fixtures
pnpm test:e2e:full-stack
pnpm build
```

`pnpm check`（`vp check`）依次运行 Oxfmt 格式检查、Oxlint 检查和 TypeScript 7 类型检查；lint、格式和测试配置都在 `vite.config.ts` 的 `lint`、`fmt`、`test` 块中。Oxlint 经 `jsPlugins` 加载 `@stylexjs/eslint-plugin` 与 `oxlint-anti-slop`，生成文件（`lib/generated/`、Wails 绑定、路由树）不参与 lint。`pnpm knip` 检查未使用的文件、依赖与导出（生成的 `lib/generated/api-schema.tsx` 除外），CI 要求结果为空。Oxlint 开启 anti-slop 的 `no-unnecessary-type-widening` 与 `no-typeof-outside-guards`：外部数据用 `lib/type-guards.tsx` 的类型守卫或 Zod 校验，查表用 `Map`。lint 警告与错误都必须为零。

测试边界：

- `unit-jsdom` 通过 `tests/setup.tsx` 注册 handler-free MSW server；每个套件显式安装所需领域场景，未声明请求直接失败。
- `component-browser` 运行需要真实 Chromium 的原生语义与动效套件；普通组件行为仍留在 jsdom。动效套件验证真实退出生命周期、分区标题过渡与抽屉焦点归还；键盘套件验证 Dialog、Menu、Select、Popover 与单选组的焦点、Esc 和方向键行为。
- `fixture-chromium` 构建后在 `127.0.0.1:3100` 启动 `vp preview` 并使用页面路由 fixture；`full-stack-chromium` 构建 `dist/`，启动实际 Go 服务、临时 SQLite 与代理到该服务的 `vp preview`，目录内禁止请求拦截。
- Playwright 本地零重试；CI 最多一次重试以保留 trace/video，并通过 `failOnFlakyTests` 让 retry-pass 仍失败。
- 覆盖率排除生成代码和 `components/ui/`，只作独立诊断，不设置完成阈值。

实现变更应运行与影响范围相称的检查；API facade、认证或生成契约变化时至少执行生成检查、类型检查和相关测试。统一的 `fast`、`integration`、`e2e-smoke`、`all`、`diagnostics` 命令及 CI 报告路径见[测试系统](../../../docs/guides/testing.md)。
