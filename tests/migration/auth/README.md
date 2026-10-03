# Authentication migration observations

`settings-oracle.rs` links the unchanged Rust storage crate. `export-settings.mjs`
freezes its responses to explicit compatibility cases; `export-urls.mjs` freezes
responses to input strings from `github.com/nlnwa/whatwg-url` v0.6.2's
`testdata/IdnaTestV2.json` and `testdata/urltestdata.json`. Source and corpus hashes
are embedded in the fixtures. Expected application results come from Rust, not
the Go implementation or the upstream URL fixture expectations.

The URL corpus originates in web-platform-tests. Its BSD license is retained in
`WPT-LICENSE.md`. Two unpaired-surrogate inputs are excluded because Rust strings
cannot represent them; the original JSON decoder rejects them. The library's old
IDNA expected results disagree with current Unicode data in 51 cases; all 3670
scalar-string application observations agree with the original Rust application.
The old upstream suite failure remains recorded; it is not reported as passing.

`secret-interop.mjs` builds the original Rust codec and verifies newly generated
Go ciphertext through stdin, including four associated-data contexts, Unicode
plaintext, wrong keys/contexts, and forbidden encoded line breaks. All keys and
credentials in this proof are synthetic.

The auth runner executes fresh application tests on Windows and WSL Linux, with
and without race detection. Its optional `--reuse-driver-proof` accepts only an
original successful auth report whose driver, module and command-helper inputs,
test arguments, log hashes, and freshly observed platform/build lists match.
Reused driver commands are explicitly marked; application checks always rerun.

The settings log-filter compatibility parser validates saved Rust EnvFilter
syntax only. Runtime log matching is owned by the later integration task.
