# simple 分词器（上游项目说明）

本仓库内置了 SQLite FTS5 `simple` 分词器的预编译二进制文件，分别用于 Windows 与 Linux。

## 上游项目

- 项目名称：[simple tokenizer](https://github.com/wangfenjin/simple)
- 作者：Wang Fenjin
- 原始仓库：https://github.com/wangfenjin/simple

上游 README 的核心说明如下：

> simple 是一个 SQLite3 FTS5 扩展，支持中文与拼音分词。
> 它实现了适用于多音字场景的微信移动端全文检索式中文分词方案，
> 同时也支持基于 cppjieba 的分词，以获得更好的短语匹配效果。

## Paper Scanner 中打包的文件

- `libs/simple-linux/libsimple-linux-ubuntu-latest/libsimple.so`
- `libs/simple-windows/libsimple-windows-x64/simple.dll`
- 各平台目录下附带的词典文件

## 在项目中的作用

后端会在打开 SQLite 数据库连接时尝试加载该扩展：

- Linux 下默认查找 `libsimple.so`
- Windows 下默认查找 `simple.dll`
- 也可以通过 `SIMPLE_TOKENIZER_PATH` 手动指定路径

如果扩展加载成功，`article_search` FTS5 虚表会使用 `simple` tokenizer，以提升中文分词效果。Paper Scanner 当前不启用拼音查询展开。

## 许可证

上游 `simple` 项目采用双许可证模式：`MIT` 或 `GPL-3.0-or-later`。
Paper Scanner 当前按 `MIT` 选项使用这些上游构建产物。

- 上游许可证文件：
  https://github.com/wangfenjin/simple/blob/master/LICENSE
