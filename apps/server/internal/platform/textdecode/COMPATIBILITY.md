# Strict text decoding data

The runtime uses the pinned Go `x/text` decoders. `gb18030.bin` contains
big-endian `(encoded unit uint32, Unicode scalar int32)` corrections derived
from encoding_rs 0.8.41. A negative scalar indicates malformed input. The Mozilla
MIT license is retained alongside the data.

The 16,552-byte table contains 2,067 two-byte corrections and two four-byte
corrections. SHA-256:
`4edf16a14b921f7cd6a4f64cd93b7384fbaf437bcdbd0f45ba1cdfca5aa17060`.

GBK labels use the same decoder as GB18030. Invalid syntax is rejected before
table lookup. Changes to the decoder or table require explicit review of the
supported encodings and malformed-input behavior.
