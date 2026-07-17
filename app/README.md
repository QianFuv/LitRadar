# LitRadar 前端包

`app/` 是 LitRadar 的 Next.js Web 客户端源码，负责登录、检索、收藏、每周更新、聚合设置中心和管理后台。生产构建输出静态 `out/`，由唯一的 `litradar serve` 应用进程直接提供；生产镜像没有独立 Next.js 进程或 Node.js 运行时。本页只说明前端包的开发边界：

- 系统进程与数据流见[系统架构](../docs/architecture.md)。
- REST 契约与认证方式见[API 参考](../docs/reference/api.md)。
- 环境变量的系统级语义见[运行配置](../docs/reference/configuration.md)。
- UI token 与组件约定见[设计系统](../docs/reference/design-system.md)。

## 工具链

CI 与前端构建阶段使用：

| 工具              | 版本                              |
| ----------------- | --------------------------------- |
| Node.js           | 24                                |
| pnpm              | 10.32.0                           |
| Next.js           | 16.2.4                            |
| React / React DOM | 19.2.3                            |
| TypeScript        | 5.x                               |
| Tailwind CSS      | 4.x                               |
| Rust              | 1.96；只在生成 OpenAPI 契约时需要 |

依赖由 `pnpm-lock.yaml` 锁定。前端状态与 UI 的主要库包括 TanStack Query、nuqs、next-themes、Radix UI、class-variance-authority 和 lucide-react。

## 本地运行

先在仓库根目录启动只监听 loopback 8001 的统一 Rust 应用；HTTP 和内嵌调度共享该进程：

```bash
cargo run --bin litradar -- serve \
  --host 127.0.0.1 \
  --port 8001 \
  --secret-key-file secrets/litradar.key
```

再在 `app/` 中运行：

```bash
corepack enable pnpm
pnpm install --frozen-lockfile
pnpm dev
```

默认浏览器入口统一为 `http://localhost:8000`。Next.js 开发服务器保留 HMR，并把 `/api/*`、`/mcp/*`、`/docs/*` 和 `/openapi.json` 代理到内部 Rust 地址 `http://localhost:8001`；浏览器不需要访问第二个端口。

- Web：`http://localhost:8000/`
- REST API：`http://localhost:8000/api`
- Swagger UI：`http://localhost:8000/docs/`
- OpenAPI JSON：`http://localhost:8000/openapi.json`
- MCP：`http://localhost:8000/mcp`

生产构建执行 `pnpm build` 并写入 `out/`。生产静态文件和后端路由由同一个 Rust 监听器提供，不使用 Next.js rewrite，也没有 `pnpm start`/`next start` 路径。

## 环境变量

| 变量                  | 默认值                  | 作用                                                                 |
| --------------------- | ----------------------- | -------------------------------------------------------------------- |
| `NEXT_PUBLIC_API_URL` | 空                      | 浏览器 API 根地址；空值使用当前 Origin                               |
| `INTERNAL_API_URL`    | `http://localhost:8001` | 仅供 `next dev` 将后端命名空间转发到内部 Rust 监听器；生产导出不使用 |

只有浏览器需要跨源直连后端时才设置 `NEXT_PUBLIC_API_URL`；该值会进入前端构建产物，此时后端必须允许对应 CORS Origin。标准开发和生产拓扑都保持同源，不需要设置它。

## 路由

| 路由                  | 访问边界 | 页面职责                                      |
| --------------------- | -------- | --------------------------------------------- |
| `/login`              | 公开     | 登录、注册和邀请码状态                        |
| `/`                   | 已登录   | 数据库/领域/期刊/日期筛选、FTS 搜索与文章列表 |
| `/weekly-updates`     | 已登录   | 按数据库和期刊浏览本周变化                    |
| `/favorites`          | 已登录   | 文件夹、收藏、批量操作与引文导出              |
| `?settings=<section>` | 已登录   | 在当前受保护页面上打开聚合设置中心            |
| `/admin`              | 管理员   | 用户、邀请码、统计、运行配置、计划任务与公告  |

除 `/login` 外，页面都位于 `app/(protected)/`，布局会通过 `AuthProvider` 恢复当前用户并把未登录访问重定向到 `/login?next=...`。`/admin` 还在页面层检查 `is_admin`。

设置中心由受保护布局全局挂载，合法分类为 `general`、`tracking`、`notifications`、`data-sources`、`account` 和 `tokens`。打开、切换与关闭设置只修改当前 URL 的 `settings` 参数，其他检索或页面状态保持不变；未知分类会被移除。`/settings` 和 `/tracking` 不再是页面路由，也不提供兼容跳转。

根布局提供统一标题模板，各页面通过服务端 page/layout 导出独立标题和描述。未知路由使用可静态导出的自定义 404 页面；普通路由错误提供重试和首页入口，根布局失败时由不依赖 Providers 的独立全局错误文档兜底。

首页侧栏顶部使用紧凑品牌栏，并把文献检索、我的收藏和每周更新收敛为一行三列的图标导航。每个图标入口都提供可访问名称、悬停提示和当前页面语义；移动端筛选 Dialog 复用同一组件。

认证完成后，所有受保护页面右下角都会显示带头像、用户名和展开提示的账号按钮。账号菜单只承载设置中心、外观主题、条件显示的管理面板入口和退出登录，不再重复页面导航；设置入口保留当前查询参数。主题选择支持跟随系统、浅色和深色，并通过 Radix Dropdown Menu 处理键盘导航、Escape、点击外部关闭和焦点归还。账号按钮位置与页面底部留白会考虑设备安全区。设置中心使用 Radix Dialog：桌面为双栏弹窗，移动端为全屏单列，文献追踪和通知分类共享同一份未保存草稿，并在关闭或离开追踪分类组前要求确认。

常规 UI chrome（页面表面、文字、边框、焦点环、默认/选中控件和导航状态）在 light/dark 下只使用黑、白与中性灰；暗色侧栏的“重置筛选”为近白底黑字。具有明确业务含义的颜色继续保留：蓝色表示信息或搜索命中，红色表示错误或危险操作，黄/琥珀表示收藏或警告，绿色表示成功。状态色必须同时配合文字、图标或 ARIA 语义，Logo 和账号头像位图保持原色。

## 目录职责

```text
app/
├── app/
│   ├── (protected)/       认证后的 App Router 页面
│   ├── login/             公开登录/注册页面
│   ├── globals.css        Tailwind、主题 token、字体和全局无障碍规则
│   ├── error.tsx          常规路由错误边界与重试入口
│   ├── global-error.tsx   根布局失败时的独立错误文档
│   ├── not-found.tsx      静态导出的自定义 404 页面
│   ├── layout.tsx         元数据、字体变量、skip link 和根 Provider
│   └── providers.tsx      Theme、nuqs、React Query 与认证上下文
├── components/
│   ├── admin/             管理后台功能卡片
│   ├── favorites/         收藏页视图与 view model
│   ├── feature/           检索、文章详情、侧栏和全局用户菜单
│   ├── settings/          聚合设置中心、分类内容与设置组件
│   ├── tracking/          追踪设置内容与共享 view model
│   └── ui/                Radix/CVA 基础组件
├── lib/
│   ├── api/               按 auth/index/favorites/tracking/admin 拆分的 facade
│   ├── generated/         OpenAPI JSON 与生成的 TypeScript schema
│   ├── api.tsx            API facade 的公共导出
│   ├── api-contract.tsx   安全敏感响应的运行时校验
│   ├── auth-context.tsx   Cookie 会话恢复与认证操作
│   └── browser-storage.tsx
└── tests/
    ├── *.test.tsx         Vitest/jsdom/MSW 测试
    └── e2e/               Playwright 本地 fixture 流程
```

新增业务逻辑时优先放入对应 feature 目录；可复用的视觉原语放入 `components/ui/`。仓库约定所有 TypeScript 源文件使用 `.tsx`，即使文件不包含 JSX。

## 客户端状态

| 状态                 | 所有者                                        |
| -------------------- | --------------------------------------------- |
| 后端查询与 mutation  | TanStack Query                                |
| 搜索、筛选和周报选择 | nuqs URL query state                          |
| 登录用户             | `AuthProvider` + `GET /api/auth/me`           |
| 当前数据库           | `localStorage: litradar:v1:selected_database` |
| 搜索历史             | `localStorage: litradar:v1:search_history`    |
| 主题                 | next-themes 的 `class` 属性与系统偏好         |

浏览器 API 请求默认 `credentials: include`，登录令牌只存在后端设置的 `litradar_session` HttpOnly Cookie 中。设置中心创建的 Bearer 访问令牌用于外部客户端，不作为前端登录态存入 Web Storage。

升级前的浏览器命名空间不会被读取、复制或清理。升级后用户需要重新登录，数据库选择和搜索历史会在新命名空间中重新建立。

Web Storage helper 会容忍 SSR、隐私模式和 quota 错误；调用方不应假定写入必然成功。

站点图标和侧栏标识使用 `public/litradar-logo.png` 本地静态资源，不依赖第三方图片域名。

## API 契约

Rust API 注解是 REST schema 的来源。前端生成物：

- `lib/generated/openapi.json`
- `lib/generated/api-schema.tsx`

后端路由、DTO 或 OpenAPI 注解变化后运行：

```bash
pnpm generate:api
```

CI 使用：

```bash
pnpm generate:api:check
```

生成命令会运行 Rust `litradar openapi` 子命令、生成 TypeScript 类型并格式化产物。不要手工修改 `lib/generated/`。

`lib/api/` 提供面向页面的请求 facade；`lib/api-contract.tsx` 对认证、秘密配置、计划任务和手动推送等控制面响应再做运行时校验。普通页面不应绕过共享 transport 自行复制 Cookie、Bearer、数据库选择或错误解析逻辑。

## 质量检查

与前端 CI 一致的顺序：

```bash
pnpm generate:api:check
pnpm lint
pnpm format:check
pnpm exec tsc --noEmit
pnpm test
pnpm exec playwright install --with-deps chromium
pnpm test:e2e
pnpm build
```

测试边界：

- Vitest 使用 jsdom，`tests/setup.tsx` 注册 MSW；`*.test.tsx` 不访问真实后端。
- Playwright 默认在 `127.0.0.1:3100` 启动隔离的 Next.js dev server；设置 `PLAYWRIGHT_BASE_URL` 时改为验证已经运行的 Rust 静态站点。`tests/e2e/` 使用页面路由 fixture，不访问真实上游服务。
- CI 对 Chromium 使用单 worker 和最多两次重试，本地保持 Playwright 默认并行度。
- 覆盖率排除生成代码和 `components/ui/`，聚焦业务 facade、组件和页面。

实现变更应运行与影响范围相称的检查；API facade、认证或生成契约变化时至少执行生成检查、类型检查和相关测试。
