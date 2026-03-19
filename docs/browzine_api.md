# BrowZine 集成说明

本文档说明仓库中 `scripts/browzine/client.py` 的当前实现方式，以及它在索引流程中的角色。

## 一、实现概览

当前客户端类为：

- `BrowZineAPIClient`

它负责：

- 为不同 `library_id` 获取并缓存 BrowZine API token
- 访问期刊、期次、文章等 JSON 接口
- 对瞬时失败做有限重试

基础地址：

- `https://api.thirdiron.com/v2`

默认库 ID：

- `3050`

在项目代码中对应常量：

- `DEFAULT_LIBRARY_ID = "3050"`

## 二、认证机制

BrowZine 的访问令牌不是写死的，需要先请求：

- `POST /v2/api-tokens`

请求体核心字段：

```json
{
  "libraryId": "3050",
  "returnPreproxy": true,
  "client": "bzweb",
  "forceAuth": false
}
```

客户端会把返回 token 缓存在内存里，并结合 `expires_at` 做过期判断。

### token 缓存特点

- 按 `library_id` 维度缓存
- 请求前先检查本地缓存是否可用
- 若接口返回 `401`，会强制刷新 token 后重试
- 会预留 `TOKEN_EXPIRY_BUFFER` 避免快过期 token 被继续使用

## 三、主要能力

### 1. 期刊检索

| 方法 | 作用 |
| --- | --- |
| `search_by_issn(issn, library_id)` | 按 ISSN 搜索期刊 |
| `get_journal_info(journal_id, library_id)` | 获取单个期刊详情 |

索引流程通常先根据 CSV 中的 ISSN 做 BrowZine 检索，再取详情补全元数据。

### 2. 期次与文章

| 方法 | 作用 |
| --- | --- |
| `get_issues_by_year(journal_id, year, library_id)` | 获取某期刊某年的 issue 列表 |
| `get_articles_from_issue(issue_id, library_id)` | 获取某个 issue 的文章列表 |
| `get_articles_in_press(journal_id, library_id)` | 获取 in-press 文章 |

这些方法的输出会被索引器统一转换为数据库记录。

## 四、与 CSV 和索引器的关系

CSV 行中只要 `library != -1`，索引器就会按 BrowZine 路径处理。

典型流程：

1. 从 CSV 读取：
   - `title`
   - `issn`
   - `library`
2. 用 `search_by_issn()` 找期刊
3. 用 `get_journal_info()` 补全期刊字段
4. 拉取各年份 issue
5. 拉取 issue 下文章与 in-press 文章
6. 写入 SQLite

## 五、重试策略

当前 `_get_json()` 对以下情况有内置处理：

- 网络请求异常
- `401`：刷新 token 后重试
- `429 / 500 / 502 / 503 / 504`：短暂等待后重试

这使得在 BrowZine 偶发不稳定时，索引过程不必立刻中断。

## 六、代码中的关键约束

### 1. library_id 是一等输入

当前客户端不是只面向单个固定 library，而是：

- 初始化时有 `default_library_id`
- 每次请求也显式接收 `library_id`

因此同一套代码可以服务多个 BrowZine 库。

### 2. 期刊排序与筛选发生在本地数据库

BrowZine 只负责上游抓取。用户在 API 和前端看到的：

- `scimago_rank`
- `available`
- `has_articles`
- 年份过滤

都由本地 SQLite 检索层完成，而不是 BrowZine 在线查询完成。

## 七、常见失效点

若 BrowZine 集成出现异常，优先排查：

1. token 接口是否返回结构变化
2. `expires_at` 字段格式是否变化
3. issue / article 接口路径是否调整
4. 某些字段是否从 `attributes` 中改名

## 八、调试建议

当索引结果与预期不符时，通常按以下顺序检查最有效：

1. `search_by_issn()` 是否命中正确期刊
2. `get_journal_info()` 返回的 `attributes`
3. `get_issues_by_year()` 返回的年份和 issue 数量
4. `get_articles_from_issue()` 与 `get_articles_in_press()` 的原始 payload
5. `scripts/index/transforms.py` 中的字段映射是否遗漏
