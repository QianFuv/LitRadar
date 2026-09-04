# 前端设计系统

本文档描述当前已经实现的视觉 token、基础组件、布局和无障碍约定，不是外部品牌复刻规范。实现来源：

- `app/app/globals.css`：主题、字体、圆角、阴影和全局行为
- `app/app/layout.tsx`、`app/app/providers.tsx`：根文档、主题和根级无障碍
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

| 内容                         | 字体链                               |
| ---------------------------- | ------------------------------------ |
| 正文与普通 UI                | `'JetBrainsLxgwNerdMono', monospace` |
| `code`、`kbd`、`samp`、`pre` | `'JetBrainsLxgwNerdMono', monospace` |

`JetBrainsLxgwNerdMono` 是正文、控件、标题、文章内容、设置/管理 Dialog 和代码区域的统一首选字体。`app/assets/JetBrainsLxgwNerdMono-Regular/result.css` 通过 344 个本地 WOFF2 分片提供 400 字重，`app/assets/JetBrainsLxgwNerdMono-Bold/result.css` 通过 324 个本地 WOFF2 分片提供 700 字重；两套生成样式都保留生成器、版权与 OFL/MIT 授权元数据，按 Unicode range 引用字体并为每个 `@font-face` 使用 `font-display: swap`。根布局不加载 Google 托管字体，字体链只在本地字体缺失时回退系统 `monospace`；正文与代码继续启用 `'liga' 1`，700 字重使用本地 Bold 字形。

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

Button 统一使用 `rounded-md`、禁用态 opacity、有限属性 transition 和 3px `focus-visible` ring。非 link 按钮在按下时使用 `scale(0.96)`，只在未请求 reduced motion 且未禁用时启用；`static` 可关闭按压缩放，用于搜索清空、筛选移除、收藏等高频或需要保持锚点稳定的操作。图标按钮必须提供可访问名称。

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
| Dialog            | `bg-black/50` overlay；默认居中，移动工作区侧栏使用 `placement="left"` 抽屉 |
| ScrollArea        | Radix viewport 与 10px 自定义 scrollbar                                     |
| Skeleton          | muted pulse，用于加载占位                                                   |
| StateMessage      | 紧凑的空态/错误/成功/警告表面；色彩始终配合图标、标题与 live-region role    |
| Label             | 与原生表单关联；禁用状态随 peer/group 传播                                  |

复杂表单应组合现有 primitive，不要重新实现键盘导航、焦点管理或 portal 行为。

### 聚合设置与管理中心

所有已认证页面都从当前 pathname 的 `settings` query 打开全局设置 Dialog。稳定分类为 `general`、`tracking`、`notifications`、`data-sources`、`account` 和 `tokens`；分类切换使用 replace 语义，只改这一参数，关闭时移除参数，未知值直接规范化移除。`/settings` 与 `/tracking` 不是页面路由。

桌面 `md` 及以上使用受 `90dvh` 和 1rem viewport margin 限制的大型双栏 Dialog：左侧约 240px 分类栏，右侧为固定标题和独立滚动内容。移动端使用 `h-dvh`、`w-screen` 的全屏单列布局，分类导航置于顶部并允许水平滚动，底部操作栏避开 safe area。

该响应式外壳由无业务状态的 `SectionedDialogFrame` 统一提供，包括分类导航、当前分类标题、内容滚动区、关闭控件和焦点归还；设置中心只持有 URL、追踪草稿与确认状态，不重复实现布局。

管理员通过当前受保护 pathname 的 `admin` query 打开同一外壳。稳定分类为 `overview`、`users`、`invite-codes`、`runtime-settings`、`scheduled-tasks` 和 `announcements`；六个既有管理卡片在一次 Dialog 会话内保持挂载，非当前 panel 使用原生 `hidden` 隐藏，因此切换分类不会丢失局部表单状态。`settings` 与 `admin` 互斥，打开任一中心会移除另一个参数；合法 `settings` 与手工冲突的 `admin` 同时出现时由设置中心优先并清理管理参数。未知分类和普通用户手工添加的 `admin` 会被规范化移除，不会挂载管理数据组件。

文献追踪与通知分类在两者之间切换时复用同一个 tracking view model 和草稿；保存/取消栏 sticky 在内容滚动区底部。关闭设置、浏览器返回或离开追踪分类组时，如果草稿未保存，必须先显示独立 `ConfirmDialog`。文章详情中的数据源入口必须先关闭文章 Dialog，再打开 `settings=data-sources`，不允许叠加两个 modal。Dialog 关闭后把焦点归还给仍在文档中的发起控件。

### 页面导航与账号菜单

首页侧栏顶部使用紧凑的品牌栏，品牌栏下方是一行三列导航：`Search` /“检索”对应文献检索，`Star` /“收藏”对应我的收藏，`CalendarDays` /“周报”对应每周更新。短标签必须始终可见以便快速辨认，链接同时保留完整 `aria-label`、`title` 和 `aria-current="page"` 当前页语义；桌面侧栏与移动端筛选 Dialog 复用同一导航组件。

所有受保护页面右下角使用带圆形头像、用户名和展开提示的账号 pill。账号菜单只承载四类账号级动作：打开聚合设置中心、在子菜单中选择 system/light/dark 主题、向管理员显示管理面板入口，以及使用 destructive 语义退出登录。页面级导航不应在账号菜单中重复；设置与管理链接必须保留当前 pathname 和现有 query，并用一次性标记让 Dialog 关闭后把焦点归还给账号按钮。菜单复用 Radix Dropdown Menu 的键盘导航、Escape、点击外部关闭与焦点归还行为，并避开设备 safe area。退出登录的红色属于明确的危险操作语义，不受普通 UI chrome 的中性色约束。

### 文章列表层级

文章卡片使用紧凑的单一表面：标题是第一视觉层，期刊/卷期/日期使用较小的中性元数据行，开放获取与预发表 badge 作为次级信号，摘要限制三行，明确的“查看详情”操作位于浅色底栏。标题使用均衡换行，摘要使用 `text-pretty`；窄屏 badge 排在标题下方，避免挤压长标题。卡片正文保持可选择，不把整张卡片改成按钮，也不让 badge、摘要或操作与标题竞争。hover 只改变背景，不替换共享阴影环或制造可点击整张卡片的浮起效果。

搜索框、清空、搜索、帮助、筛选移除、查看详情及收藏操作在移动端保留至少 44px 的实际命中高度，桌面为 40px；图标按钮同时保证相应宽度，不使用会重叠的伪元素扩展命中区。搜索清空即时反馈并归还输入焦点，不改变已提交查询。搜索历史与收藏选择器使用 10px 外圆角、8px padding 和 2px 内圆角，沿用共享 shadow stack。

收藏按钮为两个文字状态预留相同宽度，避免切换时推动相邻操作；星形使用 `currentColor`，仅选中状态填充，配合文字和收藏夹的 `aria-pressed` 表达状态。收藏文字使用浅色 `amber-700` / 深色 `amber-400`，不对标签或图标重播 presence 入场。

搜索的 loading、error、empty 和 results 只在列表级状态边界交叉淡入淡出；错误与空态使用 `StateMessage`。列表边界之外只保留一个即时更新的 live region，分页加载复用该语义状态，避免退出中的视觉表面成为陈旧播报。长文章结果不逐卡应用 presence、layout 或 stagger，继续保留 `content-visibility-card` 与 Intersection Observer sentinel。

## 布局与响应式

项目使用组件内的 Tailwind mobile-first breakpoint，没有单独维护一套自定义 breakpoint 表。

当前主要模式：

- 首页使用动态 viewport 高度；桌面在 `md` 显示 `w-80` 固定侧栏，移动端改为左侧 Dialog 筛选器。
- 首页搜索头 sticky，搜索与结果正文限制为 `max-w-4xl`。
- 有已应用筛选时，结果区先显示“共找到 N 条结果”，再显示相对 `results-scroll-container` 悬浮的筛选摘要；总数随文章列表滚动离开，筛选摘要保持可操作。
- 聚合设置中心与管理面板按上面的共享响应式 Dialog 规则布局；收藏使用 `max-w-5xl`，周报使用 `max-w-6xl`。
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
- 默认 Dialog 关闭按钮为移动端 44×44px、桌面 40×40px，标题预留关闭区并使用均衡换行，描述使用 `text-pretty`。
- Dialog 动画和普通 transition 受全局 reduced-motion 规则约束；关闭动画期间仍由 Radix 保持 portal 与焦点归还生命周期。

颜色不能作为唯一状态信号；状态文本、图标或 ARIA 语义应与颜色同时存在。

### 动效约定

根 `Providers` 在主题 Provider 内提供 `MotionProvider`。它使用 Motion 的 `LazyMotion`、`domAnimation` 和 strict 模式；业务组件只能从 `components/ui/motion.tsx` 使用本地 presence、variant、transition 和 `m` 元素封装，不直接导入 Motion 包。页面级 presence 默认 `initial={false}`，避免首次服务端渲染与 hydration 产生无意义入场。

动效保持快速、克制且可预测：

| 场景             | 进入                              | 退出                              |
| ---------------- | --------------------------------- | --------------------------------- |
| Overlay          | opacity，160ms                    | opacity，120ms                    |
| 居中 Dialog      | opacity + 6px + scale 0.98，200ms | opacity + 4px + scale 0.98，140ms |
| 左侧移动抽屉     | `translateX(-100%)`，220ms        | `translateX(-100%)`，160ms        |
| Popover / Select | opacity + 4px + scale 0.98，140ms | opacity + 4px + scale 0.98，110ms |
| 普通状态切换     | 120–180ms，进入曲线               | 120–140ms，退出曲线               |

进入使用 `cubic-bezier(0.16, 1, 0.3, 1)`，退出使用 `cubic-bezier(0.4, 0, 1, 1)`。只对会被 React 条件卸载且需要退出生命周期的状态使用 JS presence；Radix portal 使用共享 CSS animation，继续由 Radix 管理键盘、Escape、点击外部与焦点。长结果列表不逐项错峰，key 必须来自稳定业务标识，控件只声明需要过渡的属性，不使用 `transition-all`。

系统不使用 bounce、spring、parallax、drag、layout animation 或 `domMax`。`prefers-reduced-motion: reduce` 和测试 override 会移除空间位移、延迟与 JS presence 时长；CSS animation/transition 保持 0.01ms，以便状态完成而不制造可感知运动。

## 修改准则

新增或调整 UI 时：

1. 先选择语义 token，再决定是否需要场景色。
2. 先扩展现有 primitive variant，再创建新的基础组件。
3. 同时验证 system/light/dark；允许的局部 Tailwind 状态色必须有必要的 dark 对应。
4. 保留键盘焦点、可访问名称、状态角色和 reduced-motion 行为。
5. 使用现有页面宽度、动态 viewport、`w-80` 侧栏与 mobile-first 组合，不引入未经实现验证的布局规则或固定像素高度差。
6. 运行前端包说明中的格式、类型、测试和构建检查。

token 或基础组件变化时更新本页；单个页面的业务规则应留在代码和对应功能文档中。
