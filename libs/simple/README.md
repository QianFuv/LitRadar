# SQLite simple tokenizer

Content schema v9 uses FTS5 `tokenize = 'simple 0'`. Chinese text is indexed as character sequences, so ordinary phrase MATCH can find a short phrase inside a longer title or abstract. The `0` explicitly disables pinyin aliases, including initials. LitRadar never invokes `simple_query()` or `jieba_query()` and does not install Jieba dictionaries.

Search still uses parameterized, table-wide `article_search MATCH ?`; simple/advanced query modes, filters, ordering and pagination are unchanged. Search-only Unicode normalization preserves covered Latin accent/case behavior and punctuation word boundaries. Advanced query operands are normalized without changing FTS operators or column names. Native simple segmentation may split alphanumeric terms differently from unicode61; migration validates the new search contract rather than requiring all old result sets to be identical. Canonical article text and identifiers are not normalized or rewritten.

## Native builds and locations

- Windows x64 development uses `libs/simple-windows/libsimple-windows-x64/simple.dll` (SHA256 `89cd063db0c01ba97bb78f61fb500488b8c14f7cd162962a728dc20836ef0108`).
- Linux development and CI run `node scripts/build-simple-tokenizer.mjs`, producing `target/simple-tokenizer/libsimple.so`. CMake and a C++14 compiler are required.
- Docker builds the library for its actual amd64/arm64 target and installs `/usr/lib/litradar/libsimple.so` plus the C++ runtime.
- Packaged native executables may carry the matching library beside the executable. Only fixed executable/package/build locations are searched; database contents and the configured data directory cannot select an extension.

Linux source is pinned to upstream commit `45db071ba8043ffe8a2e5dfe41f9d68fb477576c`, with source archive SHA256 `d60f39ecad1f4fcf46485810708353777224ddc3829b7c9de865034277481e61`. Builds set `SIMPLE_WITH_JIEBA=OFF`, `BUILD_SQLITE3=OFF`, `BUILD_TEST_EXAMPLE=OFF` and `BUILD_STATIC=OFF`. The bundled pinyin resource remains part of upstream's library but is not used by `simple 0`.

## Existing databases

Exact v6/v7/v8 content schemas retain `unicode61` and remain readable without the native extension. Startup does not silently rebuild them. To enable Chinese matching in an existing database, stop writers and run the documented offline `admin index optimize-storage --confirm-index-maintenance` operation. It streams canonical rows into a v9 candidate and validates identities before replacement. Backups and old binaries must be paired with the corresponding old index files for rollback.

Missing or incompatible native code is an explicit error for v9; there is no fallback to unicode61. SQLite extension loading is enabled only while registering the trusted library, then disabled again.

## License

Upstream: https://github.com/wangfenjin/simple/tree/45db071ba8043ffe8a2e5dfe41f9d68fb477576c

LitRadar selects the MIT option from upstream's MIT OR GPL-3.0-or-later licensing. Copyright and permission text are distributed in `third-party/Simple-LICENSE.txt`.
