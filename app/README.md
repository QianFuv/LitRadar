# LitRadar 前端包

`app/` 是 LitRadar 的 Next.js Web 客户端源码，负责登录、检索、收藏、每周更新、文献追踪、个人设置和管理后台。生产构建输出静态 `out/`，由唯一的 `litradar serve` 应用进程直接提供；生产镜像没有独立 Next.js 进程或 Node.js 运行时。本页只说明前端包的开发边界：

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

| 路由              | 访问边界 | 页面职责                                      |
| ----------------- | -------- | --------------------------------------------- |
| `/login`          | 公开     | 登录、注册和邀请码状态                        |
| `/`               | 已登录   | 数据库/领域/期刊/日期筛选、FTS 搜索与文章列表 |
| `/weekly-updates` | 已登录   | 按数据库和期刊浏览本周变化                    |
| `/favorites`      | 已登录   | 文件夹、收藏、批量操作与引文导出              |
| `/tracking`       | 已登录   | 追踪文件夹、AI/PushPlus 设置与手动推送        |
| `/settings`       | 已登录   | 账号、改密、邀请码、访问令牌与 CNKI 会话      |
| `/admin`          | 管理员   | 用户、邀请码、统计、运行配置、计划任务与公告  |

除 `/login` 外，页面都位于 `app/(protected)/`，布局会通过 `AuthProvider` 恢复当前用户并把未登录访问重定向到 `/login?next=...`。`/admin` 还在页面层检查 `is_admin`。

认证完成后，所有受保护页面右下角都会显示全局用户菜单，可直接访问首页、收藏、追踪、每周更新和账号设置；管理员还会看到管理面板入口。菜单同时提供跟随系统、浅色和深色三种主题偏好，并通过 Radix Dropdown Menu 处理键盘导航、Escape、点击外部关闭和焦点归还。菜单位置与页面底部留白会考虑设备安全区。

## 目录职责

```text
app/
├── app/
│   ├── (protected)/       认证后的 App Router 页面
│   ├── login/             公开登录/注册页面
│   ├── globals.css        Tailwind、主题 token、字体和全局无障碍规则
│   ├── layout.tsx         元数据、字体变量、skip link 和根 Provider
│   └── providers.tsx      Theme、nuqs、React Query 与认证上下文
├── components/
│   ├── admin/             管理后台功能卡片
│   ├── favorites/         收藏页视图与 view model
│   ├── feature/           检索、文章详情、侧栏和全局用户菜单
│   ├── settings/          个人设置功能
│   ├── tracking/          追踪页视图与 view model
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

浏览器 API 请求默认 `credentials: include`，登录令牌只存在后端设置的 `litradar_session` HttpOnly Cookie 中。设置页创建的 Bearer 访问令牌用于外部客户端，不作为前端登录态存入 Web Storage。

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
