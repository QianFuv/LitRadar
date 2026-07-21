# 测试系统

本文档是 LitRadar 测试分层、数据契约、执行命令和诊断策略的唯一完整说明。日常开发先选择能证明行为的最低层；只有跨进程、浏览器或容器装配本身是风险时，才上移到更昂贵的层。

## 五层模型

| 层级                    | 主要工具与位置                                                                                             | 适用问题                                                                                         | 不应承担                          |
| ----------------------- | ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | --------------------------------- |
| 1. 单元                 | Rust 模块内 `#[cfg(test)]`；Vitest 的纯 helper 测试                                                        | 解析、规范化、状态机、序列化、错误映射和纯业务规则                                               | HTTP、真实浏览器或进程装配        |
| 2. 契约与适配器         | Rust crate 集成测试、临时 SQLite、Axum router、MCP、loopback transport、共享 JSON 场景                     | 路由/存储/迁移/Provider/CLI 边界，以及真实响应与 OpenAPI 场景的一致性                            | 页面交互和浏览器语义              |
| 3. 前端功能组件         | `app/tests/*.test.tsx` 的 Vitest/jsdom/MSW；仅必要时使用 `app/tests/browser-components/*.browser.test.tsx` | 页面状态、mutation、缓存、路由、错误呈现；焦点、Clipboard、IntersectionObserver 等浏览器原生语义 | 完整后端或部署拓扑                |
| 4. 浏览器 fixture smoke | `app/tests/e2e/local-fixtures.spec.tsx` 的 Playwright Chromium                                             | 少量跨页面 UI、可访问导航、主题和响应式关键流；API 由显式页面 fixture 提供                       | 后端、Cookie、SQLite 持久化真实性 |
| 5. 真实系统边界         | `app/tests/e2e/full-stack/`、`crates/litradar/tests/`、`scripts/container-smoke.mjs`                       | 前端导出 → 实际 Rust listener → 临时 SQLite，以及真实进程、信号、镜像安全和清理                  | 组合式边界条件枚举                |

一个改动可以由多层共同拥有，但每条业务规则必须有一个最低充分所有者。高层 smoke 只证明关键装配，不复制低层的全部输入组合。

## 放置与处置规则

- Rust 私有实现规则放在所属模块旁；跨 crate 公共行为放在对应 crate 的 `tests/`；真实 `litradar` 进程边界放在 `crates/litradar/tests/`。
- REST 路由场景放在 `crates/litradar-api/src/tests/`；MCP 协议和工具行为留在 `mcp.rs` 的现有测试所有者中。
- 普通前端行为放在 `app/tests/*.test.tsx`。只有 jsdom 无法忠实提供的浏览器 API 或事件链，才进入 `browser-components/`。
- fixture Playwright 只放在 `local-fixtures.spec.tsx`；真实后端 Playwright 只放在 `e2e/full-stack/`，且禁止 `page.route`、`context.route`、`route.fulfill`、`route.abort` 等拦截。
- 跨栈稳定 JSON 放在 `testdata/scenarios/api/`；运行时生成物、随机凭据和数据库快照不得签入该目录。
- 不为视觉整齐批量移动测试。审阅现有用例时使用以下处置：
  - **保留**：在正确层证明唯一可观察行为。
  - **加强**：意图有效，但缺少结果、状态或失败断言。
  - **重写**：依赖实现细节、隐式 fixture、catch-all 场景，或在 jsdom 中错误模拟浏览器行为。
  - **合并/删除**：更强所有者已通过，原用例没有唯一断言。
  - **新增**：已实现功能、权限、失败或装配边界没有所有者。

修复缺陷时保留一个能在旧行为上失败的回归测试。测试名称应说明业务意图，而不是复述函数名。

## OpenAPI、共享场景与 MSW

Rust OpenAPI 注解是 HTTP schema 的唯一来源：

```text
Rust OpenAPI annotations
  -> app/lib/generated/openapi.json
  -> app/lib/generated/api-schema.tsx
  -> typed scenario imports and MSW handlers
```

当前共享语料仅包含登录、文章页、每周更新、掩码通知设置和标准错误五类稳定响应。规则如下：

1. Rust router 测试构造真实临时存储，取得响应并与共享 JSON 比较；必要的时间字段只可规范化为固定哨兵值。
2. TypeScript 通过生成的 `components['schemas']` 类型约束 JSON；认证、秘密设置等敏感响应还必须经过 `app/lib/api-contract.tsx` 的现有运行时解析器。
3. 共享 JSON 不得包含 token、Cookie、密码、凭据、绝对路径、随机 ID 或运行时生成时间，也不得成为第二套 schema。
4. 修改路由、DTO 或 OpenAPI 注解后，在 `app/` 运行 `pnpm generate:api:check`；不要手工编辑 `lib/generated/`。
5. 不增加跨 Rust/TypeScript 的共享 helper、Pact 或另一套 schema 生成器来替代该链路。

MSW 的全局 server 不安装登录态或业务默认值，并以 `onUnhandledRequest: 'error'` 拒绝未声明请求。测试从 `app/tests/mocks/handlers/` 显式安装 auth、discovery/index、favorites、tracking 或 admin 场景 bundle；单个失败场景只覆盖必要 handler，测试结束后由公共 setup 重置。这样每个请求依赖在套件中可见，不会由其他测试留下的状态暗中满足。

## 浏览器边界

Vitest Browser Mode 目前只拥有三类 jsdom 保真缺口：

- Dialog 的 pointer、Escape 和焦点归还；
- 真实 Clipboard API 的成功与不可用反馈；
- 原生 IntersectionObserver、布局、滚动和事件链。

普通渲染、表单、缓存、错误、mutation 和路由状态仍由 jsdom 拥有。新增 Browser Mode 用例前，应先证明所需 Web API、布局或事件顺序在 jsdom 中不可忠实验证。

Playwright 有两个独立角色：

- `fixture-chromium` 保留 7 条快速 UI smoke；它启动隔离 Next.js dev server，并显式拦截 API。
- `full-stack-chromium` 串行运行 3 条关键旅程；它先构建静态前端，再启动实际 `litradar serve` 和临时 SQLite/index，验证 HttpOnly 会话、搜索/收藏持久化、管理员 mutation、权限、退出和匿名拒绝。

全栈 fixture 由 marker 保护，只能写入 OS 临时根；不提供生产测试端点，不读取真实 `data/`、`secrets/` 或外部凭据，也不访问 Crossref、OpenAlex、Semantic Scholar、ZJLIB、CNKI、AI 或 PushPlus。

## 功能所有权矩阵

| 功能                   | 最低充分所有者                                                                                                   | 契约/适配器所有者                                                              | 真实关键边界                                                        |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ | ------------------------------------------------------------------- |
| 认证与账户             | `litradar-auth` 单元测试；`login-page`、`auth-context`、`user-menu`、`account-settings`                          | `litradar-api` auth route、rate-limit 和共享 login/error 场景                  | full-stack 登录、HttpOnly 会话、退出、匿名 401 与非管理员 403       |
| 检索、文章、周报与公告 | `litradar-index`；`results-list`、`search-filter-ui`、`article-dialog`、`weekly-updates`、`announcements-dialog` | index/weekly REST、文章访问、共享 article/weekly 场景                          | full-stack SQLite 文章检索；管理员公告写入后 refetch                |
| 收藏与导出             | `favorite-flow`、`favorite-checks`、citation helper                                                              | favorites REST 的 folder、batch、BibTeX/RIS/EndNote 与用户隔离                 | full-stack 收藏后刷新仍持久化                                       |
| 追踪与投递             | `tracking-page`、`tracking-polling`；`litradar-worker` delivery/retry                                            | tracking REST、notify/push CLI 的本地空变更状态                                | fixture tracking push smoke；真实 CLI 子命令进程边界                |
| 管理后台               | `admin-users`、`admin-mutations`、`admin-announcements`、runtime secret suites                                   | admin REST、调度存储、密码/邀请码/角色/运行设置校验                            | full-stack 用户角色、邀请码和公告 mutation 持久化                   |
| REST 与 MCP            | API route/unit suites；MCP initialize/index/favorites tool suites                                                | OpenAPI 完整路由检查、共享场景、临时 router/storage                            | `crates/litradar/tests/service.rs` 的实际 listener；full-stack REST |
| CLI 与统一服务         | `litradar-cli` parser/runner；`litradar` runtime 单元测试                                                        | `crates/litradar/tests/cli.rs` 的真实二进制副作用                              | `service.rs` 启动、readiness、认证、信号、端口与临时根清理          |
| Provider 与索引        | `litradar-domain`、`litradar-provider`、`litradar-index`                                                         | source fixture、生产 ZJLIB transport 的 bounded loopback、迁移/identity/outbox | 真实 CLI index 对本地已完成 catalog 的恢复                          |
| 调度与 worker          | worker scheduler/delivery/AI/PushPlus fixture 测试；runtime 协调测试                                             | 租约、时区、超时、取消、去重、持久状态和安全日志                               | scheduler run-once 启动实际类型化子命令并等待结果                   |
| 容器运行时             | Dockerfile/Compose 静态检查                                                                                      | `scripts/container-smoke.mjs` 的 HTTP 与 inspect 断言                          | CI 对将要推送的同一镜像 ID 执行硬化启动和完整清理                   |

## 统一命令

先安装前端依赖和本地执行工具：

```bash
cd app
corepack enable pnpm
pnpm install --frozen-lockfile
cd ..
cargo install cargo-nextest --version 0.9.137 --locked
cargo install cargo-sort --version 2.1.4 --locked
```

需要覆盖率时另装固定版本：

```bash
cargo install cargo-llvm-cov --version 0.8.7 --locked
```

所有统一命令都从仓库根运行，任一子步骤失败即停止，并转发 SIGINT/SIGTERM：

| 命令                                | 精确职责                                                                                                             |
| ----------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `node scripts/test.mjs fast`        | Cargo workspace 的 library/binary 测试，加 Vitest jsdom；不构建浏览器，不运行 E2E。                                  |
| `node scripts/test.mjs integration` | cargo-nextest workspace、独立 doctest、OpenAPI 生成幂等和共享前端 API contract。                                     |
| `node scripts/test.mjs e2e-smoke`   | 构建/导出前端，并只运行 3 条真实后端 Chromium 关键旅程。                                                             |
| `node scripts/test.mjs all`         | Rust/前端静态检查、完整 nextest/doctest、jsdom、Browser Mode、7 条 fixture smoke、3 条 full-stack smoke 和前端构建。 |
| `node scripts/test.mjs diagnostics` | 分别生成 Rust 和前端覆盖率报告；不应用百分比阈值。                                                                   |

在命令末尾加 `--ci` 会选择 nextest CI profile、固定报告路径和浏览器 CI 诊断策略。`--ci` 不是额外测试层，也不会让统一脚本自行重试失败命令。

## 直接命令

聚焦排障时可直接运行所属框架：

```bash
# Rust
cargo test -p <crate> --locked <filter>
cargo nextest run --workspace --locked
cargo test --workspace --doc --locked

# Frontend (from app/)
pnpm generate:api:check
pnpm test:unit
pnpm test:browser-components
pnpm test:e2e:fixtures
pnpm test:e2e:full-stack
pnpm build
```

`cargo test --workspace --locked` 保留为完整计划/发布前的一次 Cargo 兼容门禁；普通 PR 的 Rust 主执行器是 nextest，doctest 单独运行。安装浏览器依赖使用 `cd app && pnpm exec playwright install --with-deps chromium`。

容器边界必须测试将要发布的确切本地镜像：

```bash
docker build --tag litradar:test .
node scripts/container-smoke.mjs litradar:test
```

探针要求 readiness、根页和 OpenAPI 成功，镜像 ID 不变，用户非 root，根文件系统只读，drop 全部 capability，启用 no-new-privileges，使用 tmpfs、可写数据卷和只读密钥挂载；成功或失败后都要删除容器、卷、监听端口和 marker 保护的密钥临时根。

## 报告与失败诊断

`--ci` 使用以下固定路径：

| 报告                                               | 路径                                                                 |
| -------------------------------------------------- | -------------------------------------------------------------------- |
| nextest JUnit                                      | `target/nextest/ci/junit.xml`                                        |
| Vitest jsdom JUnit                                 | `app/test-results/vitest/junit.xml`                                  |
| Vitest Browser Mode JUnit                          | `app/test-results/vitest-browser/junit.xml`                          |
| Browser Mode 截图                                  | `app/test-results/browser-components/screenshots/`                   |
| fixture Playwright JUnit/trace/screenshot/video    | `app/test-results/playwright-fixtures/`                              |
| fixture Playwright HTML                            | `app/playwright-report/fixtures/`                                    |
| full-stack Playwright JUnit/trace/screenshot/video | `app/test-results/playwright-full-stack/`                            |
| full-stack Playwright HTML                         | `app/playwright-report/full-stack/`                                  |
| Rust coverage                                      | `target/llvm-cov/html/`、`target/llvm-cov/lcov.info`                 |
| Frontend coverage                                  | `app/coverage/`、`app/coverage/lcov.info`                            |
| Container smoke                                    | `test-results/container-smoke/summary.json` 和失败时的 `failure.log` |

CI 的 artifact upload 使用 `if: always()`。失败时先看 workflow summary 的层级状态和时长，再看 JUnit 的失败 owner；浏览器问题打开对应 HTML，并使用失败截图、第一次重试的 trace/video。容器问题先看安全清理摘要，再看已脱敏的尾部日志。报告目录均为生成物，不应提交。

## 重试、flaky 与时长

- Rust 本地和 CI 都是零重试；`flaky-result = "fail"`。CI 不 fail-fast，以便收集完整失败；本地默认在首个失败停止。单测超过 60 秒会被标记 slow，连续三个周期后终止。
- Playwright 本地零重试；CI 最多一次重试，只用于取得 trace/video。`failOnFlakyTests` 已启用，因此 retry-pass 仍使 CI 失败，不能作为稳定完成证据。
- Vitest 和统一脚本不自动重试。不要通过重复运行直到通过来关闭缺陷。
- backend、frontend 和 container workflow summary 记录各层状态与时长；nextest JUnit 记录测试时长。只有持续数据证明某层成为瓶颈后，才讨论 shard/partition。

## 覆盖率策略

覆盖率是独立、信息性的诊断，不是完成标准：

- `.github/workflows/test-diagnostics.yaml` 每周一 02:00 UTC（`0 2 * * 1`）或手动运行；不由 pull request 触发。
- Rust 与前端报告分开保留，不合并百分比，也不比较两种语言。
- 不设置总量、changed-line 或目录阈值；百分比变化不单独使任务通过或失败。
- 使用报告定位无所有者的高风险行为，再以功能、权限、失败和装配断言补测试。

## 延后工具及采用条件

| 工具/策略                         | 当前决定 | 重新评估条件                                                                  |
| --------------------------------- | -------- | ----------------------------------------------------------------------------- |
| Pact 或另一套消费者契约           | 延后     | 出现独立部署、独立版本的消费者，并先定义兼容/破坏策略。                       |
| Testcontainers                    | 延后     | 自动测试引入 SQLite/临时文件无法替代的外部数据库、队列或服务。                |
| Firefox/WebKit 门禁               | 延后     | 产品声明支持对应浏览器，或真实缺陷/使用数据要求覆盖。                         |
| Playwright shard / Rust partition | 延后     | 多次 workflow 时长证明明确瓶颈，并能在不隐藏 flaky 的前提下稳定拆分。         |
| mutation testing                  | 延后     | 稳定核心规则仍发生断言逃逸，且有可接受的定时预算与结果 owner。                |
| property testing                  | 延后     | 解析、身份或状态机存在可表达的不变量，示例测试已证明覆盖不足。                |
| fuzzing                           | 延后     | 面向不可信输入的 parser 暴露安全风险，并具备 corpus、资源上限和崩溃归档流程。 |

这些工具必须解决已观测的问题，不能仅因测试数量或覆盖率数字而引入。
