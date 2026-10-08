# SQLite simple 分词器

当前内容库 v9 使用 FTS5 的 `tokenize = 'simple 0'`。中文按字符序列索引，普通短语 MATCH 可以匹配标题或摘要中的短语；参数 `0` 显式关闭拼音及首字母别名。LitRadar 不调用 `simple_query()` 或 `jieba_query()`，也不安装 Jieba 词典。

搜索仍通过参数化的全表 `article_search MATCH ?` 执行，简单与高级查询模式、过滤、排序和分页保持原有接口。只有检索投影和查询文本进行 Unicode 规范化，以保留所覆盖的拉丁重音、大小写和标点分词行为；高级操作数的规范化不改变 FTS 运算符或列名。原生 simple 对字母数字词项的切分可能与 unicode61 不同，迁移验证新的检索契约，不要求所有旧查询结果完全相同。规范文章原文与标识符不被改写。

<a id="native-builds-and-locations"></a>

## Static builds and compatibility oracles

Windows and Linux use the same pinned Simple source, compiled against the unchanged SQLite driver's headers with `SQLITE_CORE`. The executable directly initializes each admitted connection; it does not discover or load `simple.dll` or `libsimple.so`.

Run `node scripts/build-simple-tokenizer.mjs` from the repository root. It requires curl, tar, CMake and a Go-compatible C++14 compiler; Windows additionally requires MinGW and Ninja. The result is `target/simple-tokenizer/libsimple.a`. Build scripts include archive, header, source and compiler identities in Go's cache flags. Direct Go commands must also use `simpleBuildEnvironment`, as shown in the [development guide](../../docs/guides/development.md).

Source revision: `45db071ba8043ffe8a2e5dfe41f9d68fb477576c`; archive SHA-256: `d60f39ecad1f4fcf46485810708353777224ddc3829b7c9de865034277481e61`. The adapter excludes Jieba, SQLite and example targets and includes both CMRC resource objects. The embedded pinyin resource does not enable aliases with `simple 0`.

The existing v0.7.1 Windows DLL and Linux SO remain unchanged as historical inputs, excluded from packages. Their SHA-256 values are `27c700ca34cd5935ff934459f1f9c107cdefef7e54250de5a1c6646e05f21a4f` and `5493821c973a0dfee1afeb270ff5efbef49b4002c117433a3a2203393874a991`. `--compatibility-oracle` copies the verified Windows DLL or builds the previous Linux source-pinned shared library separately. Tests compare actual token positions, MATCH/highlight and existing v9 append/update/delete behavior; they never rebuild the old index to hide a source transition.

Static Simple removes its runtime library requirement. Linux still needs the documented system C/C++ libraries; Obscura, Poppler and metadata catalogs remain separate deployment inputs.

<a id="existing-databases"></a>

## 现有数据库

精确 v6/v7/v8 内容库保留 unicode61，无需原生扩展即可读取；启动不会静默重建。要为旧库启用中文短语匹配，必须先停止写入者并准备已验证备份，再执行[离线索引优化](../../docs/reference/cli.md#索引存储优化)。命令把规范记录流式重建到 v9 候选库，并在替换前验证身份。回滚时，旧二进制必须配合它支持的旧版索引备份。

Static registration failures are explicit; v9 does not fall back to unicode61. Registration is opt-in per physical connection, leaves plain/auth roles unchanged and never enables SQL extension loading.

<a id="license"></a>

## 许可证与来源

Historical precompiled oracles originate from [Simple v0.7.1](https://github.com/wangfenjin/simple/releases/tag/v0.7.1)；Docker 和本地 Linux 源码构建使用 [simple 固定上游提交](https://github.com/wangfenjin/simple/tree/45db071ba8043ffe8a2e5dfe41f9d68fb477576c)。LitRadar 在上游的 MIT OR GPL-3.0-or-later 双许可证中选择 MIT；版权与授权原文随项目分发于 [Simple-LICENSE.txt](../../docs/third-party/Simple-LICENSE.txt)。
