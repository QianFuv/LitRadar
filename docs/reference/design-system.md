# 前端设计系统

本文档描述当前已经实现的视觉 token、基础组件、布局和无障碍约定，不是外部品牌复刻规范。实现来源：

- `app/app/globals.css`：主题、字体、圆角、阴影和全局行为
- `app/app/layout.tsx`、`app/app/providers.tsx`：字体变量、主题和根级无障碍
- `app/components/ui/*.tsx`：基础组件 variants
- `app/app/(protected)/*.tsx` 与业务组件：真实布局和响应式用法

前端开发流程见[前端包说明](../../app/README.md)。

## 系统层次

| 层                  | 职责                                                                             |
| ------------------- | -------------------------------------------------------------------------------- |
| 语义 token          | light/dark 的背景、文字、交互、边框、状态色                                      |
| Tailwind theme 映射 | 把 CSS 变量映射为 `bg-background`、`text-foreground`、`border-border` 等 utility |
| UI primitives       | Button、Card、Dialog、Input、Select 等可复用外观和交互                           |
| 业务组件            | 搜索、文章、收藏、追踪和管理页面的组合与少量场景色                               |

优先复用语义 token 和 UI primitive。业务层只有搜索高亮、成功、警告和错误等局部状态可以使用明确的 Tailwind palette，而且必须同时处理深色主题；结构文字、背景、边框和 hover 不得使用 slate 或十六进制场景色。

## 字体

有效字体链由 `globals.css` 决定：

| 内容                         | 字体链                                                         |
| ---------------------------- | -------------------------------------------------------------- |
| 正文与普通 UI                | `'Maple Mono Normal NL CN', var(--font-geist-sans), monospace` |
| `code`、`kbd`、`samp`、`pre` | `'Maple Mono Normal NL CN', var(--font-geist-mono), monospace` |

Maple Mono 是当前正文和代码的首选字体。根布局通过 `next/font/google` 加载 Geist Sans 与 Geist Mono，但它们只提供回退变量，不是有效首选字体。两条字体链都启用 `'liga' 1`。

字号和字重主要使用 Tailwind utility，由具体组件按信息层级选择；项目没有一套独立的固定 display typography scale。旧版外部品牌标题尺寸、负字距和三字重规则不属于项目约束。

## 主题

ThemeProvider 使用 `attribute="class"`、`defaultTheme="system"` 和 `enableSystem`：

- 首次渲染跟随操作系统主题。
- 认证后的全局用户菜单提供 system/light/dark 单选项。
- light/dark 是持久化的显式选择；system 会继续响应系统偏好变化。
- 依赖当前主题的控件必须在客户端快照可用后渲染，避免 hydration 差异。
- 根 viewport 声明 `colorScheme: light dark`，并分别给出白色与黑色 theme color。

### 核心颜色

| Token                                  | Light                 | Dark                  | 用途              |
| -------------------------------------- | --------------------- | --------------------- | ----------------- |
| `--background`                         | `#ffffff`             | `#000000`             | 页面背景          |
| `--foreground`                         | `#171717`             | `#ededed`             | 主文字            |
| `--card` / `--popover`                 | `#ffffff`             | `#000000`             | 浮层与卡片        |
| `--primary`                            | `#171717`             | `#ededed`             | 主操作            |
| `--primary-foreground`                 | `#ffffff`             | `#000000`             | 主操作文字        |
| `--secondary` / `--muted` / `--accent` | `#fafafa`             | `#111111`             | 次级和 hover 表面 |
| `--muted-foreground`                   | `#666666`             | `#888888`             | 辅助文字          |
| `--destructive`                        | `#ff5b4f`             | `#ff5b4f`             | 破坏性操作        |
| `--info` / `--info-foreground`         | `#ebf5ff` / `#0068d6` | `#00152b` / `#ebf5ff` | 信息 Badge        |
| `--border` / `--input`                 | `#ebebeb`             | `#333333`             | 边框和输入轮廓    |
| `--ring` / `--sidebar-ring`            | `#171717`             | `#ededed`             | 普通键盘焦点      |
| `--sidebar-primary`                    | `#171717`             | `#ededed`             | 侧栏选中状态      |
| `--sidebar-primary-foreground`         | `#ffffff`             | `#000000`             | 侧栏主状态文字    |

默认 UI chrome 包括页面/浮层表面、结构文字、边框、普通焦点环、默认或选中控件和导航状态；这些值在 light/dark 下都必须是黑、白或中性灰。侧栏的 background、foreground、primary、accent、border 和 ring 使用独立语义 token；滚动条也使用灰阶 light/dark token。

色相只用于有明确业务含义的状态：蓝色用于信息和搜索命中，红色用于错误与危险操作，黄/琥珀用于收藏和警告，绿色用于成功。每个状态还必须有文字、图标、边框差异或 ARIA role，颜色不能成为唯一信号。`litradar-logo.png` 及账号头像属于位图内容资产，不受 chrome 灰阶约束。当前没有图表组件，因此不保留未使用的 chart token；新增颜色时必须先归入上述语义边界。

## 圆角

基础值为 `--radius: 6px`。Tailwind 映射：

| Utility token | 计算值 |
| ------------- | -----: |
| `radius-sm`   |    2px |
| `radius-md`   |    4px |
| `radius-lg`   |    6px |
| `radius-xl`   |   10px |
| `radius-2xl`  |   14px |
| `radius-3xl`  |   18px |
| `radius-4xl`  |   22px |

Badge 和滚动条使用全圆角；个别紧凑控件使用 Tailwind 自带的 `rounded-xs`。圆角由组件语义决定，不存在“主按钮禁止 pill”之类的额外品牌规则。

## 阴影与边框

项目保留两个历史命名的 shadow token：

| Token                  | Light                                  | Dark                                   |
| ---------------------- | -------------------------------------- | -------------------------------------- |
| `--shadow-vercel-ring` | `rgba(0, 0, 0, 0.08) 0 0 0 1px`        | `rgba(255, 255, 255, 0.14) 0 0 0 1px`  |
| `--shadow-vercel-card` | 外环 + 2px/8px 轻阴影 + `#fafafa` 内环 | 亮外环 + 两层黑色阴影 + 半透明白色内环 |

`shadow-vercel-ring` 用于 outline Button、Badge、Input、Select 等紧凑控件；`shadow-vercel-card` 用于 Card 和可见的 skip link。

阴影环没有取代所有 CSS border。当前实现明确混用两者：

- Dialog 使用 `border` 与 `shadow-lg`。
- 页面分隔、列表项、虚线空状态和表单反馈使用 `border-*`。
- Card 默认使用 shadow stack，业务 hover 可能替换为局部 shadow。
- 控件的 invalid 状态可以增加 destructive border。

真实边框仍是系统组成部分；根据布局分隔、焦点、状态和 elevation 选择边框或阴影。

## 基础组件

### Button

Variants：

- `default`：primary 实底
- `destructive`：破坏性实底
- `outline`：背景 + shadow ring
- `secondary`：次级表面
- `ghost`：仅 hover 表面
- `link`：文本链接

Sizes：

- `xs`、`sm`、`default`、`lg`
- `icon-xs`、`icon-sm`、`icon`、`icon-lg`

Button 统一使用 `rounded-md`、禁用态 opacity、有限属性 transition 和 3px `focus-visible` ring。图标按钮必须提供可访问名称。

### Badge

Badge 默认全圆角，支持 `default`、`secondary`、`destructive`、`outline`、`ghost` 和 `link`。默认 variant 使用 `info`/`info-foreground` token；状态语义可以选择其他 variant。

### Card

Card 使用 card token、`rounded-lg`、`shadow-vercel-card`、24px 外层纵向 padding 和统一 header/content/footer 结构。业务组件可以调整间距、hover 背景或 shadow，但应复用 Card 的语义结构。聚合设置中心是明确例外：内部使用 `SettingsSection` 的无阴影分隔行，避免在大 Dialog 中继续嵌套整组 Card elevation。

### 表单和浮层

| 组件              | 实现约定                                                                    |
| ----------------- | --------------------------------------------------------------------------- |
| Input             | 36px 高、shadow ring、移动端 16px 字号、`md` 后 14px、3px focus ring        |
| Checkbox / Switch | Radix 状态属性驱动颜色、焦点和禁用态                                        |
| Select / Popover  | Radix portal，使用 popover token 与 shadow；内容限制在 viewport 内          |
| Dialog            | `bg-black/50` overlay，内容默认距视口 1rem、`md:max-w-4xl`、border + shadow |
| ScrollArea        | Radix viewport 与 10px 自定义 scrollbar                                     |
| Skeleton          | muted pulse，用于加载占位                                                   |
| Label             | 与原生表单关联；禁用状态随 peer/group 传播                                  |

复杂表单应组合现有 primitive，不要重新实现键盘导航、焦点管理或 portal 行为。

### 聚合设置中心

所有已认证页面都从当前 pathname 的 `settings` query 打开全局设置 Dialog。稳定分类为 `general`、`tracking`、`notifications`、`data-sources`、`account` 和 `tokens`；分类切换使用 replace 语义，只改这一参数，关闭时移除参数，未知值直接规范化移除。`/settings` 与 `/tracking` 不是页面路由。

桌面 `md` 及以上使用受 `90dvh` 和 1rem viewport margin 限制的大型双栏 Dialog：左侧约 240px 分类栏，右侧为固定标题和独立滚动内容。移动端使用 `h-dvh`、`w-screen` 的全屏单列布局，分类导航置于顶部并允许水平滚动，底部操作栏避开 safe area。

文献追踪与通知分类在两者之间切换时复用同一个 tracking view model 和草稿；保存/取消栏 sticky 在内容滚动区底部。关闭设置、浏览器返回或离开追踪分类组时，如果草稿未保存，必须先显示独立 `ConfirmDialog`。文章详情中的数据源入口必须先关闭文章 Dialog，再打开 `settings=data-sources`，不允许叠加两个 modal。Dialog 关闭后把焦点归还给仍在文档中的发起控件。

### 页面导航与账号菜单

首页侧栏顶部使用紧凑的品牌栏，品牌栏下方是一行三列的图标导航：`Search` 对应文献检索、`Star` 对应我的收藏、`CalendarDays` 对应每周更新。图标入口必须同时提供 `aria-label`、`title`、`sr-only` 文本和 `aria-current="page"` 当前页语义；桌面侧栏与移动端筛选 Dialog 复用同一导航组件。

所有受保护页面右下角使用带圆形头像、用户名和展开提示的账号 pill。账号菜单只承载四类账号级动作：打开聚合设置中心、在子菜单中选择 system/light/dark 主题、向管理员显示管理面板入口，以及使用 destructive 语义退出登录。页面级导航不应在账号菜单中重复；设置链接必须保留当前 pathname 和现有 query。菜单复用 Radix Dropdown Menu 的键盘导航、Escape、点击外部关闭与焦点归还行为，并避开设备 safe area。退出登录的红色属于明确的危险操作语义，不受普通 UI chrome 的中性色约束。

## 布局与响应式

项目使用组件内的 Tailwind mobile-first breakpoint，没有单独维护一套自定义 breakpoint 表。

当前主要模式：

- 首页使用动态 viewport 高度；桌面在 `md` 显示 `w-80` 固定侧栏，移动端改为左侧 Dialog 筛选器。
- 首页搜索头 sticky，搜索与结果正文限制为 `max-w-4xl`。
- 聚合设置中心按上面的响应式 Dialog 规则布局；管理和收藏使用 `max-w-5xl`，周报使用 `max-w-6xl`。
- 页面 padding 常从 `p-4` / 紧凑间距过渡到 `sm:p-6`。
- 收藏页在 `md` 从单列变成 `280px + 1fr`。
- 表单按钮和选择器通常在移动端占满宽度，`sm` 后恢复行内布局。
- Dialog 默认适应窄屏；首页筛选器在移动端覆盖左侧并在 `md` 隐藏。
- 全视口 shell 使用 `h-dvh` / `min-h-dvh`，避免移动浏览器工具栏遮挡。
- 浮动账号 pill 使用设备安全区定位；页面或内部滚动区预留对应底部净空。
- 长列表使用独立滚动容器、Intersection Observer 和命名的 `content-visibility-card`、`content-visibility-row`、`content-visibility-table-row`、`content-visibility-filter-row` 类。每个类都编码匹配内容类型的 intrinsic block size，业务组件不重复任意 CSS 声明。
- 固定头部与滚动正文使用 `flex`、`min-h-0` 和 `flex-1` 分配高度，不使用依赖头部像素值的 `calc(100% - …)`。

页面宽度、列数、导航和 breakpoint 以现有 utility 与组件行为为准，不从旧设计稿推导新规则。

## 无障碍与动效

根级保障：

- `<html lang="zh-CN">`。
- 页面首个可聚焦元素是“跳到主要内容”链接；各页面主区域使用 `id="main-content"`。
- `.skip-link` 平时视觉隐藏，`focus-visible` 时显示。
- `prefers-reduced-motion: reduce` 将 animation/transition 缩短到 0.01ms，并关闭平滑滚动。
- 滚动条同时提供 Firefox 与 WebKit 样式。

组件保障：

- 交互控件使用可见的 ring；不要用 `outline: none` 后不补焦点样式。
- 图标按钮使用 `aria-label` 或 `sr-only` 文本。
- 加载、成功与错误反馈使用 `role="status"` / `role="alert"`。
- 展开、选中和当前状态使用 Radix data attributes 或对应 ARIA 属性。
- Dialog、DropdownMenu、Select、Popover、Checkbox 和 Switch 复用 Radix 的键盘与焦点行为。
- Dialog 动画和普通 transition 受全局 reduced-motion 规则约束。

颜色不能作为唯一状态信号；状态文本、图标或 ARIA 语义应与颜色同时存在。

## 修改准则

新增或调整 UI 时：

1. 先选择语义 token，再决定是否需要场景色。
2. 先扩展现有 primitive variant，再创建新的基础组件。
3. 同时验证 system/light/dark；允许的局部 Tailwind 状态色必须有必要的 dark 对应。
4. 保留键盘焦点、可访问名称、状态角色和 reduced-motion 行为。
5. 使用现有页面宽度、动态 viewport、`w-80` 侧栏与 mobile-first 组合，不引入未经实现验证的布局规则或固定像素高度差。
6. 运行前端包说明中的格式、类型、测试和构建检查。

token 或基础组件变化时更新本页；单个页面的业务规则应留在代码和对应功能文档中。
