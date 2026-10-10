# Static Simple adapter

This adapter compiles the verified upstream archive described in `source.json`
without modifying its sources. It uses the application's go-sqlite3 headers and
`SQLITE_CORE`, so all SQLite calls resolve to the driver's own implementation.
Jieba is disabled. Both CMRC resource objects are included in `libsimple.a`, with
the upstream `contrib/pinyin.txt` key preserved.

Run `node scripts/build-simple-tokenizer.mjs` before native Go builds. Windows
requires curl, tar, MinGW GCC/G++, CMake and Ninja; Linux requires curl, tar,
GCC/G++, CMake and Make.
The compiler must match Go's cgo toolchain. Generated source, archive and input
identities remain under ignored `target/`. Go entrypoints include the verified
native input digest in their cgo flags to invalidate cached compilation when
archive or header bytes change.

`--compatibility-oracle` additionally prepares the trusted old Windows v0.7.1 DLL
or the previous Linux source-built extension for comparison tests. These files
are test inputs and are never included in production distributions. The selected
upstream license and notice remain in `docs/third-party/Simple-LICENSE.txt`.
