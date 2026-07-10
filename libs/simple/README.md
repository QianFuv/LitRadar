# SQLite `simple` 分词扩展

本目录记录 Paper Scanner 内置的 SQLite FTS5 `simple` 扩展、发现规则与许可证边界。

## 打包文件

| 平台        | 扩展                                                           |
| ----------- | -------------------------------------------------------------- |
| Windows x64 | `libs/simple-windows/libsimple-windows-x64/simple.dll`         |
| Linux       | `libs/simple-linux/libsimple-linux-ubuntu-latest/libsimple.so` |

平台目录同时包含扩展需要的词典。

## 运行时发现

常规进程根据 `project-root` 查找上述固定路径。数据库 schema 初始化的兼容路径还会从主数据库位置和当前工作目录向上查找祖先目录中的 `libs/`。

当前实现不提供环境变量路径覆盖。

扩展存在且能被 SQLite 加载时，`article_search` 使用：

```sql
tokenize = 'simple'
```

文件缺失、平台不受支持或扩展加载失败时，schema 仍会创建，并回退到 SQLite FTS5 默认 tokenizer。已经创建的 FTS 表不会仅因后来加入扩展而自动改变 tokenizer；需要通过受支持的迁移或重建流程处理。

Paper Scanner 不额外实现拼音查询展开。

## 上游与许可证

扩展来自 [wangfenjin/simple](https://github.com/wangfenjin/simple)，支持中文与拼音分词。上游采用 `MIT OR GPL-3.0-or-later` 双许可证；本项目按 MIT 选项使用打包产物。

上游许可证见 [LICENSE](https://github.com/wangfenjin/simple/blob/master/LICENSE)。
