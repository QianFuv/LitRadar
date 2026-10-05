# Original logging Unicode tables

Unicode 16.0.0 ranges and simple case folding from locked regex-syntax 0.8.11. Embedded as compatibility data; no runtime dependency. This prevents changes to Go's Unicode release from changing persisted log-filter semantics.

Upstream: https://github.com/rust-lang/regex/tree/regex-syntax-0.8.11/regex-syntax/src/unicode_tables

Generated tables.json SHA-256: bb96f6a2785cb2d66bf9fc482144261a5d7898dcff545d08b9df012510c347fa

Source hashes:

- general_category.rs: 9488e3721f7c2ae20e1b77fcff9a59b4ed8f22954b8645ea6d8592eac1856423
- perl_word.rs: 30f073baae28ea34c373c7778c00f20c1621c3e644404eff031f7d1cc8e9c9e2
- perl_space.rs: ec9bb22ed7e99feef292249c7e6f4673ee0af9635d4d158f93923494c14cd5ed
- perl_decimal.rs: 6a59143db81a0bcaf0e8d0af265e711d1a6472e1f091ee9ee4377da5d5d0cd1f
- case_folding_simple.rs: 7622c7f7f03ac0dc2f2bcd51c81a217d64de0cc912f62f1add5f676603a02456
