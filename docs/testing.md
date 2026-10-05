# 测试系统

本文档是 LitRadar 测试分层、数据契约、执行命令和诊断策略的唯一完整说明。日常开发先选择能证明行为的最低层；只有跨进程、浏览器或容器装配本身是风险时，才上移到更昂贵的层。

## 五层模型

| 层级                    | 主要工具与位置                                                                                             | 适用问题                                                                                         | 不应承担                          |
| ----------------------- | ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | --------------------------------- |
| 1. 单元                 | Go 包内 `*_test.go`；Vitest 的纯 helper 测试                                                               | 解析、规范化、状态机、序列化、错误映射和纯业务规则                                               | HTTP、真实浏览器或进程装配        |
| 2. 契约与适配器         | Go 集成测试、临时 SQLite、net/http 与 httptest、MCP、loopback transport、共享 JSON 场景                    | 路由/存储/迁移/Provider/CLI 边界，以及真实响应与 OpenAPI 场景的一致性                            | 页面交互和浏览器语义              |
| 3. 前端功能组件         | `app/tests/*.test.tsx` 的 Vitest/jsdom/MSW；仅必要时使用 `app/tests/browser-components/*.browser.test.tsx` | 页面状态、mutation、缓存、路由、错误呈现；焦点、Clipboard、IntersectionObserver 等浏览器原生语义 | 完整后端或部署拓扑                |
| 4. 浏览器 fixture smoke | `app/tests/e2e/local-fixtures.spec.tsx` 的 Playwright Chromium                                             | 少量跨页面 UI、可访问导航、主题和响应式关键流；API 由显式页面 fixture 提供                       | 后端、Cookie、SQLite 持久化真实性 |
| 5. 真实系统边界         | `app/tests/e2e/full-stack/`、`cmd/litradar/`、`tests/container-smoke.mjs`                                  | 前端导出 → 实际 Go listener → 临时 SQLite，以及真实进程、信号、镜像安全和清理                    | 组合式边界条件枚举                |

一个改动可以由多层共同拥有，但每条业务规则必须有一个最低充分所有者。高层 smoke 只证明关键装配，不复制低层的全部输入组合。

## 放置与处置规则

- Go 规则与适配器测试放在所属包旁的 `*_test.go`；真实 `litradar` 进程边界放在 `cmd/litradar/main_test.go`。
- REST 路由场景放在 `internal/api/`；MCP 协议和工具测试放在 `internal/mcp/` 与 `internal/platform/mcpcompat/`。
- 普通前端行为放在 `app/tests/*.test.tsx`。只有 jsdom 无法忠实提供的浏览器 API 或事件链，才进入 `browser-components/`。
- fixture Playwright 放在 `local-fixtures.spec.tsx`；真实后端 Playwright 只放在 `e2e/full-stack/`，且禁止 `page.route`、`context.route`、`route.fulfill`、`route.abort` 等拦截。
- 跨栈稳定 JSON 放在 `tests/data/scenarios/api/`；运行时生成物、随机凭据和数据库快照不得签入该目录。
- 不为视觉整齐批量移动测试。审阅现有用例时使用以下处置：
  - **保留**：在正确层证明唯一可观察行为。
  - **加强**：意图有效，但缺少结果、状态或失败断言。
  - **重写**：依赖实现细节、隐式 fixture、catch-all 场景，或在 jsdom 中错误模拟浏览器行为。
  - **合并/删除**：更强所有者已通过，原用例没有唯一断言。
  - **新增**：已实现功能、权限、失败或装配边界没有所有者。

修复缺陷时保留一个能在旧行为上失败的回归测试。测试名称应说明业务意图，而不是复述函数名。

## OpenAPI、共享场景与 MSW

Go 的 `internal/openapi/` 声明与真实路由绑定共同约束 HTTP schema：

```text
Go OpenAPI definitions + operations + route bindings
  -> app/lib/generated/openapi.json
  -> app/lib/generated/api-schema.tsx
  -> typed scenario imports and MSW handlers
```

当前共享语料仅包含登录、文章页、每周更新、掩码通知设置和标准错误五类稳定响应。规则如下：

1. Go 路由测试构造真实临时存储，通过原版迁移观测语料和响应断言验证兼容性；前端消费这里的共享 JSON。当前 Go 测试不直接加载这些前端共享场景，不能将两条证据链混为一谈。
2. TypeScript 通过生成的 `components['schemas']` 类型约束 JSON；认证、秘密设置等敏感响应还必须经过 `app/lib/api-contract.tsx` 的现有运行时解析器。
3. 共享 JSON 不得包含 token、Cookie、密码、凭据、绝对路径、随机 ID 或运行时生成时间，也不得成为第二套 schema。
4. 修改路由、DTO 或 OpenAPI 声明后，在 `app/` 运行 `pnpm generate:api:check`；不要手工编辑 `lib/generated/`。
5. 不增加跨 Go/TypeScript 的共享 helper、Pact 或另一套 schema 生成器来替代该链路。

MSW 的全局 server 不安装登录态或业务默认值，并以 `onUnhandledRequest: 'error'` 拒绝未声明请求。测试从 `app/tests/mocks/handlers/` 显式安装 auth、discovery/index、favorites、tracking 或 admin 场景 bundle；单个失败场景只覆盖必要 handler，测试结束后由公共 setup 重置。这样每个请求依赖在套件中可见，不会由其他测试留下的状态暗中满足。

## 浏览器边界

Vitest Browser Mode 覆盖 jsdom 无法忠实验证的行为：

- Dialog 的 pointer、Escape 和焦点归还；
- 真实 Clipboard API 的成功与不可用反馈；
- 原生 IntersectionObserver、布局、滚动和事件链；
- 真实退出动效生命周期与减少动态效果偏好。

普通渲染、表单、缓存、错误、mutation 和路由状态仍由 jsdom 拥有。新增 Browser Mode 用例前，应先证明所需 Web API、布局或事件顺序在 jsdom 中不可忠实验证。

Playwright 有两个独立角色：

- `fixture-chromium` 运行快速 UI 冒烟测试；它启动隔离 Next.js dev server，并显式拦截 API。
- `full-stack-chromium` 串行运行真实后端关键旅程；它先构建静态前端，再启动实际 `litradar serve` 和临时 SQLite/index，验证 HttpOnly 会话、搜索/收藏持久化、管理员 mutation、权限、退出和匿名拒绝。

全栈 fixture 由 marker 保护，只能写入 OS 临时根；不提供线上测试端点，不读取真实 `data/`、`secrets/` 或外部凭据，也不访问 Crossref、OpenAlex、Semantic Scholar、ZJLIB、CNKI、AI 或 PushPlus。

## 功能所有权矩阵

| 功能                   | 最低充分所有者                                                                                                   | 契约/适配器所有者                                                              | 真实关键边界                                                              |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ | ------------------------------------------------------------------------- |
| 认证与账户             | `internal/auth` 单元测试；`login-page`、`auth-context`、`user-menu`、`account-settings`                          | `internal/api` auth route、rate-limit 和共享 login/error 场景                  | full-stack 登录、HttpOnly 会话、退出、匿名 401 与非管理员 403             |
| 检索、文章、周报与公告 | `internal/index`；`results-list`、`search-filter-ui`、`article-dialog`、`weekly-updates`、`announcements-dialog` | index/weekly REST、文章访问、共享 article/weekly 场景                          | full-stack SQLite 文章检索；管理员公告写入后 refetch                      |
| 收藏与导出             | `favorite-flow`、`favorite-checks`、citation helper                                                              | favorites REST 的 folder、batch、BibTeX/RIS/EndNote 与用户隔离                 | full-stack 收藏后刷新仍持久化                                             |
| 追踪与投递             | `tracking-page`、`tracking-polling`；`internal/delivery` / `internal/scheduler` delivery/retry                   | tracking REST、notify/push CLI 的本地空变更状态                                | fixture tracking push smoke；真实 CLI 子命令进程边界                      |
| 管理后台               | `admin-users`、`admin-mutations`、`admin-announcements`、runtime secret suites                                   | admin REST、调度存储、密码/邀请码/角色/运行设置校验                            | full-stack 用户角色、邀请码和公告 mutation 持久化                         |
| REST 与 MCP            | API route/unit suites；MCP initialize/index/favorites tool suites                                                | OpenAPI 完整路由检查、共享场景、临时 router/storage                            | `cmd/litradar/main_test.go` 的实际 listener；full-stack REST              |
| CLI 与统一服务         | `internal/cli` parser/runner；`internal/runtime` 单元测试                                                        | `cmd/litradar/main_test.go` 的真实二进制副作用                                 | `cmd/litradar/main_test.go` 启动、readiness、认证、信号、端口与临时根清理 |
| Provider 与索引        | `internal/domain`、`internal/provider`、`internal/index`                                                         | source fixture、实际 ZJLIB transport 的 bounded loopback、迁移/identity/outbox | 真实 CLI index 对本地已完成 catalog 的恢复                                |
| 调度与 worker          | worker scheduler/delivery/AI/PushPlus fixture 测试；runtime 协调测试                                             | 租约、时区、超时、取消、去重、持久状态和安全日志                               | scheduler run-once 启动实际类型化子命令并等待结果                         |
| 容器运行时             | Dockerfile/Compose 静态检查                                                                                      | `tests/container-smoke.mjs` 的 HTTP 与 inspect 断言                            | CI 对将要推送的同一镜像 ID 执行硬化启动和完整清理                         |

征稿领域的最低充分测试分别位于[领域规则](../internal/domain/cfp/oracle_test.go)、[来源解析](../internal/cfp/oracle_test.go)、[持久化](../internal/storage/cfp/oracle_test.go)、[API](../internal/api/cfp_test.go)和[前端状态](../app/tests/cfp-tracking.test.tsx)。原文与日期状态由后端测试证明，前端验证来源语言展示、分页、失败和过期选择响应；跨栈刷新由真实后端场景验证。测试数量以当前套件和运行报告为准，不在文档中重复维护。

## 统一命令

先安装 Go 1.27.1、CGO 所需的 C 编译器、Node.js 24 和前端锁定依赖。Linux 先运行 `node scripts/build-simple-tokenizer.mjs`；Windows 使用仓库提供的 DLL。Go 检查固定 `CGO_ENABLED=1`、`GOWORK=off`、`GOENV=off`、空 `GOFLAGS` 和 `GOTOOLCHAIN=go1.27.1`。

```bash
corepack enable pnpm
pnpm --dir app install --frozen-lockfile
node tests/test.mjs all
node scripts/check-go.mjs
```

所有统一命令从仓库根运行，任一子步骤失败即停止，并转发 SIGINT/SIGTERM：

| 命令                              | 精确职责                                                                                                                                          |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `node tests/test.mjs fast`        | Go `-short -count=1` 与 Vitest jsdom；当前仍包含真实进程测试                                                                                      |
| `node tests/test.mjs integration` | Go 常规全包测试、OpenAPI 幂等和前端 API contract suite                                                                                            |
| `node tests/test.mjs e2e-smoke`   | 构建静态前端和 Go 应用/fixture，运行真实后端 Chromium 旅程                                                                                        |
| `node tests/test.mjs all`         | Go 格式、module verify、vet，前端 lint/format/typecheck，integration、jsdom、Browser Mode、fixture 与 full-stack                                  |
| `node tests/test.mjs diagnostics` | Go 与前端分别生成无阈值覆盖率报告                                                                                                                 |
| `node scripts/check-go.mjs`       | Windows/Linux 后端发布检查：工具链、native 输入、格式、模块/补丁完整性、vet、无缓存 regular/race，以及 SDK/SQLite 自身模块和根模块的 regular/race |

`all` 不包含 race 或替换依赖包测试；后两者由 `check-go.mjs` 提供。`--ci` 设置 CI 环境与前端 JUnit 路径，不生成 Go JUnit，也不切换 nextest。安装浏览器依赖使用 `pnpm --dir app exec playwright install --with-deps chromium`。

聚焦 Go 检查使用 `go test -count=1 -mod=readonly -tags sqlite_fts5,sqlite_dbstat ./相关包`；涉及并发时增加 `-race`。前端可在 `app/` 单独运行 `pnpm generate:api:check`、`pnpm test:unit`、`pnpm test:browser-components`、`pnpm test:e2e:fixtures` 或 `pnpm test:e2e:full-stack`。

容器边界必须测试将要发布的确切本地镜像：

```bash
docker build --provenance=false --tag litradar:test .
node tests/container-smoke.mjs litradar:test
```

探针先用隔离数据库写入 `secure_cookies=true`，再以 `--require-secure-cookies` 重启同一镜像。它要求 readiness、Docker health、根页、OpenAPI 和 auth Header 成功，镜像 ID 不变，UID/GID 为 `10001:10001`，根文件系统只读，drop 全部 capability，启用 no-new-privileges，只发布 loopback，`/tmp` 含 `noexec,nosuid,nodev`，只有数据卷持久可写，密钥卷只读。成功或失败后都要删除容器、卷和监听端口。

Compose 配置的静态边界也必须单独验证：解析结果只有 `litradar` 服务，使用 `latest` 镜像，只向宿主机 loopback 发布 8000 端口，并保留只读根文件系统等安全选项：

```bash
docker compose \
  -f docker-compose.yml \
  config --format json
```

## 发布检查

发布工作流等待后端和前端检查，并对 amd64 与 arm64 实际镜像分别运行容器冒烟测试，通过后发布双架构 manifest。arm64 执行需要原生主机或已配置的模拟器。仓库不再提供安全扫描工作流或本地扫描入口。

## 报告与失败诊断

`--ci` 使用以下固定路径：

| 报告                                               | 路径                                                                     |
| -------------------------------------------------- | ------------------------------------------------------------------------ |
| Go 发布检查                                        | `test-results/go/results.json`、各项 `.log` 与 `native-inputs.json`      |
| Vitest jsdom JUnit                                 | `app/test-results/vitest/junit.xml`                                      |
| Vitest Browser Mode JUnit                          | `app/test-results/vitest-browser/junit.xml`                              |
| Browser Mode 截图                                  | `app/test-results/browser-components/screenshots/`                       |
| fixture Playwright JUnit/trace/screenshot/video    | `app/test-results/playwright-fixtures/`                                  |
| fixture Playwright HTML                            | `app/playwright-report/fixtures/`                                        |
| full-stack Playwright JUnit/trace/screenshot/video | `app/test-results/playwright-full-stack/`                                |
| full-stack Playwright HTML                         | `app/playwright-report/full-stack/`                                      |
| Go coverage                                        | `target/go-coverage/coverage.out`、`target/go-coverage/index.html`       |
| Frontend coverage                                  | `app/coverage/`、`app/coverage/lcov.info`                                |
| Container smoke                                    | `test-results/container-smoke/summary.json` 和失败时的 `failure.log`     |
| Container release                                  | workflow artifact `container-release`，含 Compose 解析结果与容器冒烟报告 |

CI 的 artifact upload 使用 `if: always()`。失败时先看 workflow summary 的层级状态和时长，再看 JUnit 的失败 owner；浏览器问题打开对应 HTML，并使用失败截图、第一次重试的 trace/video。容器问题先看安全清理摘要，再看已脱敏的尾部日志。报告目录均为生成物，不应提交。

## 重试、flaky 与时长

- Go 本地和 CI 不自动重试，`-count=1` 禁用结果缓存；后端发布脚本每条命令上限 15 分钟，失败即停止。
- Playwright 本地零重试；CI 最多一次重试，只用于取得 trace/video。`failOnFlakyTests` 已启用，因此 retry-pass 仍使 CI 失败，不能作为稳定完成证据。
- Vitest 和统一脚本不自动重试。不要通过重复运行直到通过来关闭缺陷。
- backend、frontend 和 container workflow summary 记录各层状态与时长；Go JSON 日志记录测试时长。只有持续数据证明某层成为瓶颈后，才讨论 shard/partition。

## 覆盖率策略

覆盖率是独立、信息性的诊断，不是完成标准：

- `.github/workflows/test-diagnostics.yaml` 每周一 02:00 UTC（`0 2 * * 1`）或手动运行；不由 pull request 触发。
- Go 与前端报告分开保留，不合并百分比，也不比较两种语言。
- 不设置总量、changed-line 或目录阈值；百分比变化不单独使任务通过或失败。
- 使用报告定位无所有者的高风险行为，再以功能、权限、失败和装配断言补测试。

## 延后工具及采用条件

| 工具/策略                           | 当前决定 | 重新评估条件                                                                  |
| ----------------------------------- | -------- | ----------------------------------------------------------------------------- |
| Pact 或另一套消费者契约             | 延后     | 出现独立部署、独立版本的消费者，并先定义兼容/破坏策略。                       |
| Testcontainers                      | 延后     | 自动测试引入 SQLite/临时文件无法替代的外部数据库、队列或服务。                |
| Firefox/WebKit 门禁                 | 延后     | 产品声明支持对应浏览器，或真实缺陷/使用数据要求覆盖。                         |
| Playwright shard / Go package split | 延后     | 多次 workflow 时长证明明确瓶颈，并能在不隐藏 flaky 的前提下稳定拆分。         |
| mutation testing                    | 延后     | 稳定核心规则仍发生断言逃逸，且有可接受的定时预算与结果 owner。                |
| property testing                    | 延后     | 解析、身份或状态机存在可表达的不变量，示例测试已证明覆盖不足。                |
| fuzzing                             | 延后     | 面向不可信输入的 parser 暴露安全风险，并具备 corpus、资源上限和崩溃归档流程。 |

这些工具必须解决已观测的问题，不能仅因测试数量或覆盖率数字而引入。
