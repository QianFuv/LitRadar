# SQLite simple 分词器

当前内容库 v9 使用 FTS5 的 `tokenize = 'simple 0'`。中文按字符序列索引，普通短语 MATCH 可以匹配标题或摘要中的短语；参数 `0` 显式关闭拼音及首字母别名。LitRadar 不调用 `simple_query()` 或 `jieba_query()`，也不安装 Jieba 词典。

搜索仍通过参数化的全表 `article_search MATCH ?` 执行，简单与高级查询模式、过滤、排序和分页保持原有接口。只有检索投影和查询文本进行 Unicode 规范化，以保留所覆盖的拉丁重音、大小写和标点分词行为；高级操作数的规范化不改变 FTS 运算符或列名。原生 simple 对字母数字词项的切分可能与 unicode61 不同，迁移验证新的检索契约，不要求所有旧查询结果完全相同。规范文章原文与标识符不被改写。

<a id="native-builds-and-locations"></a>

## 静态构建与兼容性 oracle

Windows 和 Linux 使用同一份固定版本的 Simple 源码，以 `SQLITE_CORE` 按未修改的 SQLite 驱动头文件编译。可执行文件在每个准入的连接上直接初始化分词器，不查找或加载 `simple.dll`、`libsimple.so`。

在仓库根目录运行 `node scripts/build-simple-tokenizer.mjs`。它需要 curl、tar、CMake 和与 Go 匹配的 C++14 编译器；Linux 使用 Make，Windows 另需 MinGW 和 Ninja。产物为 `target/simple-tokenizer/libsimple.a`。构建脚本把静态库、头文件、源码和编译器标识写入 Go 的缓存参数；直接运行 Go 命令时也必须使用 `simpleBuildEnvironment`，见[开发指南](../../docs/guides/development.md#原生分词器)。

源码提交：`45db071ba8043ffe8a2e5dfe41f9d68fb477576c`；源码归档 SHA-256：`d60f39ecad1f4fcf46485810708353777224ddc3829b7c9de865034277481e61`。适配构建排除 Jieba、SQLite 和示例目标，并包含两个 CMRC 资源对象。内嵌的拼音资源在 `simple 0` 下不会启用别名。

仓库中原有的 v0.7.1 Windows DLL 和 Linux SO 保持不变，不进入任何发行包，SHA-256 分别为 `27c700ca34cd5935ff934459f1f9c107cdefef7e54250de5a1c6646e05f21a4f` 和 `5493821c973a0dfee1afeb270ff5efbef49b4002c117433a3a2203393874a991`。`--compatibility-oracle` 在 Windows 上复制校验过的 DLL，在 Linux 上则用同一固定源码另行构建共享库形式的对照扩展；仓库内的 Linux SO 只作为历史记录保留，不再被脚本使用。测试比较实际 token 位置、MATCH/highlight 以及现有 v9 的追加、更新和删除行为，不通过重建旧索引掩盖源码变化。

静态链接后不再需要 Simple 运行库。Linux 仍需文档列出的系统 C/C++ 运行库；Obscura、Poppler 和期刊元数据目录仍是独立的部署输入。

<a id="existing-databases"></a>

## 现有数据库

精确 v6/v7/v8 内容库保留 unicode61，无需原生扩展即可读取；启动不会静默重建。要为旧库启用中文短语匹配，必须先停止写入者并准备已验证备份，再执行[离线索引优化](../../docs/reference/cli.md#索引存储优化)。命令把规范记录流式重建到 v9 候选库，并在替换前验证身份。回滚时，旧二进制必须配合它支持的旧版索引备份。

静态注册失败会明确报错，v9 不回退到 unicode61。注册按物理连接显式启用，不改变普通/认证连接角色，也不会开启 SQL 扩展加载。

<a id="license"></a>

## 许可证与来源

历史预编译 oracle 来自 [Simple v0.7.1](https://github.com/wangfenjin/simple/releases/tag/v0.7.1)；所有生产构建（Docker、Linux 与 Windows 发行包）都使用 [simple 固定上游提交](https://github.com/wangfenjin/simple/tree/45db071ba8043ffe8a2e5dfe41f9d68fb477576c)。LitRadar 在上游的 MIT OR GPL-3.0-or-later 双许可证中选择 MIT；版权与授权原文随项目分发于 [Simple-LICENSE.txt](../../docs/third-party/Simple-LICENSE.txt)。
