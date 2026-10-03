# Strict text decoding compatibility

The runtime uses the pinned public Go `x/text` decoders. `gb18030.bin` is a
sorted sequence of big-endian `(encoded unit uint32, Unicode scalar int32)`
corrections derived from the existing encoding_rs 0.8.41 dependency. A negative
scalar means malformed input. The Mozilla MIT license is retained alongside it.

The independent Rust exporter in `tests/migration/primitives/encoding-oracle.rs`
enumerates all 23,940 syntactically valid two-byte units and all 1,587,600
syntactically valid four-byte units. Its output starts with `LRGB1801`, followed
by strict scalar results as little-endian signed 32-bit integers. The frozen
output SHA-256 is
`d930c412b9d64eae3b126448db4f0aa65d18b8e882f639423bb2219ff7a7e66f`.
The original compiled Rust dependency SHA-256 is
`98dd7875a0cfc59ef989556e6c96e1955451678960a285269f8eebf4f25966e7`.

The `encoding-diff` command compares this oracle with x/text v0.42.0. It emits
2,067 two-byte corrections and two four-byte corrections (one preserves the
legitimately encoded replacement character). The 16,552-byte correction table
SHA-256 is `4edf16a14b921f7cd6a4f64cd93b7384fbaf437bcdbd0f45ba1cdfca5aa17060`.
The exhaustive Go test hashes results from the actual application adapter and
compares them with the independent Rust digest; it does not regenerate expected
results from Go. Normal runtime and tests do not execute Rust.

GBK labels intentionally use the same decoder as GB18030, matching the existing
WHATWG decoding behavior. Invalid syntax is rejected before table lookup. Other
encoding families need separate evidence; GB18030 coverage does not establish
their parity. In particular, Shift_JIS private-use mappings and ISO-2022-JP
state transitions remain under investigation during T02.

Updating either decoder requires repeating the differential measurement and
reviewing every changed correction. This is maintained application compatibility
data, not an upstream Go package modification.
