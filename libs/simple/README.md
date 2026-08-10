# Historical SQLite `simple` tokenizer assets

This directory retains the previously bundled SQLite FTS5 `simple` extension artifacts and their license boundary for historical reproduction. They are not dependencies of the current LitRadar content schema or runtime.

## Retained files

| Platform    | Historical extension                                               |
| ----------- | ------------------------------------------------------------------ |
| Windows x64 | `libs/simple-windows/libsimple-windows-x64/simple.dll`             |
| Linux       | `libs/simple-linux/libsimple-linux-ubuntu-latest/libsimple.so`     |

The platform directories also retain the dictionaries used by those artifacts.

## Current runtime boundary

Content schema v6 defines `article_search` with SQLite's built-in `unicode61` tokenizer. Index creation, migration validation, REST/MCP queries, and the production container do not load or require these native assets. Merely placing a DLL or shared object at a historical fixed path must not change current database behavior.

Any future importer for a database that actually declares `tokenize='simple'` must detect that schema explicitly and isolate the compatibility operation from current v6 query connections. It must not restore path-based auto-loading for every database.

LitRadar does not add pinyin query expansion to the current `unicode61` search path.

## Upstream and license

The retained extension came from [wangfenjin/simple](https://github.com/wangfenjin/simple), which supports Chinese and pinyin tokenization. Upstream uses the `MIT OR GPL-3.0-or-later` dual license; the retained project artifacts use the MIT option.

The upstream license is available at [LICENSE](https://github.com/wangfenjin/simple/blob/master/LICENSE).
