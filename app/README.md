# Paper Scanner 前端说明

`app/` 是 Paper Scanner 的 Next.js 前端工程，负责提供登录、检索、收藏、每周更新、文献追踪、系统设置与管理后台等页面。

## 当前前端职责

- 调用后端 `/api/*` 路由获取文章、期刊、收藏与管理数据
- 维护登录态与访问令牌
- 提供检索筛选、收藏导出、追踪设置、公告展示与后台管理界面
- 在 Docker 部署下通过 Next.js rewrite 将 `/api/*` 转发给 FastAPI 后端

## 技术栈

- Next.js 16
- React 19
- TypeScript 5
- Tailwind CSS 4
- Radix UI
- TanStack React Query
- nuqs
- next-themes
- lucide-react

## 启动方式

前提：

- Node.js 20+
- 推荐使用 `pnpm`
- 后端 API 已启动，默认 `http://localhost:8000`

```bash
corepack enable pnpm
pnpm install
pnpm dev
```

默认访问地址：`http://localhost:3000`

## 环境变量

| 变量                  | 默认值                                                              | 说明                                        |
| --------------------- | ------------------------------------------------------------------- | ------------------------------------------- |
| `NEXT_PUBLIC_API_URL` | 空                                                                  | 浏览器侧 API 根地址；为空时走同源 `/api/*` |
| `INTERNAL_API_URL`    | Docker 构建默认 `http://api:8000`；本地回退 `http://localhost:8000` | 用于 rewrite `/api/*` 的后端地址            |
| `HOSTNAME`            | 由运行环境决定                                                      | Next.js standalone 运行时监听地址           |

说明：

- 本地开发建议使用默认同源 `/api/*` rewrite；只有前后端分离跨源访问时才设置 `NEXT_PUBLIC_API_URL`
- Docker 构建时更关键的是 `INTERNAL_API_URL`
- 跨源浏览器请求需要后端显式配置允许的 CORS Origin，并且请求必须携带 Cookie credentials

## 页面结构

| 路由              | 说明                                               |
| ----------------- | -------------------------------------------------- |
| `/login`          | 注册、登录、邀请码判断                             |
| `/`               | 主检索页，包含筛选侧栏、搜索栏、结果列表、首页公告 |
| `/weekly-updates` | 每周更新聚合页面                                   |
| `/favorites`      | 收藏夹、导出、追踪文件夹设置                       |
| `/tracking`       | 追踪文件夹、通知设置、手动推送                     |
| `/settings`       | 个人设置、邀请码、访问令牌、修改密码               |
| `/admin`          | 管理后台：用户、邀请码、统计、定时任务、公告       |

## 目录概览

```text
app/
├── app/                      App Router 页面
│   ├── (protected)/          需要登录的页面
│   ├── login/                登录与注册页面
│   └── layout.tsx            根布局
├── components/
│   ├── admin/                管理后台组件
│   ├── feature/              搜索、收藏、每周更新等业务组件
│   └── ui/                   通用 UI 组件
├── lib/
│   ├── api.tsx               前端 API 封装
│   ├── auth-context.tsx      登录态上下文
│   ├── citation.tsx          引文文本生成
│   └── utils.tsx             前端通用辅助函数
└── next.config.ts            `/api/*` rewrite 配置
```

## 与后端的真实耦合关系

当前前端实际依赖的主要后端能力包括：

- 检索接口：`/api/articles`、`/api/journals`、`/api/issues`、`/api/meta/*`
- 每周更新与公告：`/api/weekly-updates`、`/api/announcements`
- 用户与认证：`/api/auth/*`
- 收藏与追踪：`/api/favorites/*`、`/api/tracking/*`
- 管理后台：`/api/admin/*`

首页公告展示使用 `app/components/announcements-dialog.tsx`，后台公告管理使用 `app/components/admin/announcements-card.tsx`。

## 当前认证说明

当前主流程使用后端账号体系：

- 登录：`POST /api/auth/login`
- 注册：`POST /api/auth/register`
- 获取当前用户：`GET /api/auth/me`
- 访问令牌：`/api/auth/tokens`

前端登录态由 `app/lib/auth-context.tsx` 与后端 `/api/auth/*` 共同完成。登录成功后后端设置 `HttpOnly` 的 `ps_session` Cookie，刷新页面时前端通过 `/api/auth/me` 校验并恢复用户；设置页的访问令牌由 `/api/auth/tokens` 管理，只用于外部脚本/API 客户端的 Bearer 认证。
