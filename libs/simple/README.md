# SQLite simple 分词器

当前内容库 v9 使用 FTS5 的 `tokenize = 'simple 0'`。中文按字符序列索引，普通短语 MATCH 可以匹配标题或摘要中的短语；参数 `0` 显式关闭拼音及首字母别名。LitRadar 不调用 `simple_query()` 或 `jieba_query()`，也不安装 Jieba 词典。

搜索仍通过参数化的全表 `article_search MATCH ?` 执行，简单与高级查询模式、过滤、排序和分页保持原有接口。只有检索投影和查询文本进行 Unicode 规范化，以保留所覆盖的拉丁重音、大小写和标点分词行为；高级操作数的规范化不改变 FTS 运算符或列名。原生 simple 对字母数字词项的切分可能与 unicode61 不同，迁移验证新的检索契约，不要求所有旧查询结果完全相同。规范文章原文与标识符不被改写。

<a id="native-builds-and-locations"></a>

## 原生构建与发现路径

| 环境             | 构建或加载位置                                                                                                                  |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Windows x64 开发 | `libs/simple/windows/simple.dll`                                                                                                |
| Linux x64 预编译 | `libs/simple/linux/libsimple.so`                                                                                                |
| Linux 开发与 CI  | 在仓库根运行 `node scripts/build-simple-tokenizer.mjs`，生成 `target/simple-tokenizer/libsimple.so`；需要 CMake 和 C++14 编译器 |
| Docker           | 按目标 amd64/arm64 架构构建，安装到 `/usr/lib/litradar/libsimple.so`，同时提供 C++ 运行库                                       |
| 独立原生程序     | 可在可执行文件旁打包匹配平台的库                                                                                                |

加载器只搜索固定的可执行文件、打包和编译工作区位置；数据库内容及配置的数据目录不能指定扩展。精确搜索顺序见[SQLite 连接实现](../../crates/litradar-storage/src/sqlite.rs)，Linux 构建脚本见[分词器构建](../../scripts/build-simple-tokenizer.mjs)。

仓库内预编译的 Windows DLL 和 Linux SO 来自 [Simple v0.7.1](https://github.com/wangfenjin/simple/releases/tag/v0.7.1)，SHA-256 分别为 `27c700ca34cd5935ff934459f1f9c107cdefef7e54250de5a1c6646e05f21a4f` 和 `5493821c973a0dfee1afeb270ff5efbef49b4002c117433a3a2203393874a991`。Linux 源码构建仍固定在较新的上游提交 `45db071ba8043ffe8a2e5dfe41f9d68fb477576c`，源码压缩包 SHA-256 为 `d60f39ecad1f4fcf46485810708353777224ddc3829b7c9de865034277481e61`。构建选项为 `SIMPLE_WITH_JIEBA=OFF`、`BUILD_SQLITE3=OFF`、`BUILD_TEST_EXAMPLE=OFF` 和 `BUILD_STATIC=OFF`。上游库仍包含拼音资源，但 `simple 0` 不使用它。

<a id="existing-databases"></a>

## 现有数据库

精确 v6/v7/v8 内容库保留 unicode61，无需原生扩展即可读取；启动不会静默重建。要为旧库启用中文短语匹配，必须先停止写入者并准备已验证备份，再执行[离线索引优化](../../docs/reference/cli.md#索引存储优化)。命令把规范记录流式重建到 v9 候选库，并在替换前验证身份。回滚时，旧二进制必须配合它支持的旧版索引备份。

v9 缺少或无法兼容原生库时明确失败，不回退到 unicode61。SQLite 只在注册可信库期间允许扩展加载，注册后立即关闭。

<a id="license"></a>

## 许可证与来源

预编译库来自 [Simple v0.7.1](https://github.com/wangfenjin/simple/releases/tag/v0.7.1)；Docker 和本地 Linux 源码构建使用 [simple 固定上游提交](https://github.com/wangfenjin/simple/tree/45db071ba8043ffe8a2e5dfe41f9d68fb477576c)。LitRadar 在上游的 MIT OR GPL-3.0-or-later 双许可证中选择 MIT；版权与授权原文随项目分发于 [Simple-LICENSE.txt](../../docs/third-party/Simple-LICENSE.txt)。
