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

优先复用语义 token 和 UI primitive；业务层可以为搜索高亮、错误、状态等使用明确的 Tailwind palette，但必须同时处理深色主题。

## 字体

有效字体链由 `globals.css` 决定：

| 内容                         | 字体链                                                         |
| ---------------------------- | -------------------------------------------------------------- |
| 正文与普通 UI                | `'Maple Mono Normal NL CN', var(--font-geist-sans), monospace` |
| `code`、`kbd`、`samp`、`pre` | `'Maple Mono Normal NL CN', var(--font-geist-mono), monospace` |

Maple Mono 是当前正文和代码的首选字体。根布局通过 `next/font/google` 加载 Geist Sans 与 Geist Mono，但它们只提供回退变量，不是有效首选字体。两条字体链都启用 `'liga' 1`。

字号和字重主要使用 Tailwind utility，由具体组件按信息层级选择；项目没有一套独立的固定 display typography scale。旧版外部品牌标题尺寸、负字距和三字重规则不属于项目约束。

## 主题

ThemeProvider 使用 `attribute="class"`、`defaultTheme="dark"` 和 `enableSystem={false}`：

- 首次渲染默认深色。
- 用户可以在侧栏显式切换 light/dark。
- 系统主题不会自动覆盖应用选择。
- 根 viewport 声明 `colorScheme: light dark`，并分别给出白色与黑色 theme color。

### 核心颜色

| Token                                  | Light                     | Dark      | 用途              |
| -------------------------------------- | ------------------------- | --------- | ----------------- |
| `--background`                         | `#ffffff`                 | `#000000` | 页面背景          |
| `--foreground`                         | `#171717`                 | `#ededed` | 主文字            |
| `--card` / `--popover`                 | `#ffffff`                 | `#000000` | 浮层与卡片        |
| `--primary`                            | `#171717`                 | `#ededed` | 主操作            |
| `--primary-foreground`                 | `#ffffff`                 | `#000000` | 主操作文字        |
| `--secondary` / `--muted` / `--accent` | `#fafafa`                 | `#111111` | 次级和 hover 表面 |
| `--muted-foreground`                   | `#666666`                 | `#888888` | 辅助文字          |
| `--destructive`                        | `#ff5b4f`                 | `#ff5b4f` | 破坏性操作        |
| `--border` / `--input`                 | `#ebebeb`                 | `#333333` | 边框和输入轮廓    |
| `--ring`                               | `hsla(212, 100%, 48%, 1)` | 相同      | 键盘焦点          |
| `--sidebar-primary`                    | `#171717`                 | `#0070f3` | 侧栏主状态        |

图表和滚动条也有独立 light/dark token；新增颜色时应先判断它是否属于全局语义，再决定加入 token 还是留在局部场景。

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

Badge 默认全圆角，支持 `default`、`secondary`、`destructive`、`outline`、`ghost` 和 `link`。默认 variant 是浅蓝底/蓝字，深色主题使用半透明蓝底与浅色文字；状态语义可以选择其他 variant。

### Card

Card 使用 card token、`rounded-lg`、`shadow-vercel-card`、24px 外层纵向 padding 和统一 header/content/footer 结构。业务组件可以调整间距、hover 背景或 shadow，但应复用 Card 的语义结构。

### 表单和浮层

| 组件                       | 实现约定                                                                    |
| -------------------------- | --------------------------------------------------------------------------- |
| Input                      | 36px 高、shadow ring、移动端 16px 字号、`md` 后 14px、3px focus ring        |
| Checkbox / Switch / Slider | Radix 状态属性驱动颜色、焦点和禁用态                                        |
| Select / Popover           | Radix portal，使用 popover token 与 shadow；内容限制在 viewport 内          |
| Dialog                     | `bg-black/50` overlay，内容默认距视口 1rem、`md:max-w-4xl`、border + shadow |
| ScrollArea                 | Radix viewport 与 10px 自定义 scrollbar                                     |
| Skeleton                   | muted pulse，用于加载占位                                                   |
| Label                      | 与原生表单关联；禁用状态随 peer/group 传播                                  |

复杂表单应组合现有 primitive，不要重新实现键盘导航、焦点管理或 portal 行为。

## 布局与响应式

项目使用组件内的 Tailwind mobile-first breakpoint，没有单独维护一套自定义 breakpoint 表。

当前主要模式：

- 首页占满 viewport；桌面在 `md` 显示固定侧栏，移动端改为左侧 Dialog 筛选器。
- 首页搜索头 sticky，搜索与结果正文限制为 `max-w-4xl`。
- 设置和追踪页使用 `max-w-3xl`，管理、收藏和周报使用 `max-w-5xl`。
- 页面 padding 常从 `p-4` / 紧凑间距过渡到 `sm:p-6`。
- 收藏页在 `md` 从单列变成 `280px + 1fr`。
- 表单按钮和选择器通常在移动端占满宽度，`sm` 后恢复行内布局。
- Dialog 默认适应窄屏；首页筛选器在移动端覆盖左侧并在 `md` 隐藏。
- 长文章列表使用独立滚动容器、Intersection Observer 和 `content-visibility` 降低渲染成本。

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
- Dialog、Select、Popover、Checkbox、Switch 和 Slider 复用 Radix 的键盘与焦点行为。
- Dialog 动画和普通 transition 受全局 reduced-motion 规则约束。

颜色不能作为唯一状态信号；状态文本、图标或 ARIA 语义应与颜色同时存在。

## 修改准则

新增或调整 UI 时：

1. 先选择语义 token，再决定是否需要场景色。
2. 先扩展现有 primitive variant，再创建新的基础组件。
3. 同时验证 light/dark；显式 Tailwind 颜色必须有必要的 dark 对应。
4. 保留键盘焦点、可访问名称、状态角色和 reduced-motion 行为。
5. 使用现有页面宽度与 mobile-first 组合，不引入未经实现验证的布局规则。
6. 运行前端包说明中的格式、类型、测试和构建检查。

token 或基础组件变化时更新本页；单个页面的业务规则应留在代码和对应功能文档中。
