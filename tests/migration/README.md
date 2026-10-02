# Migration compatibility evidence

The Rust baseline is `6bb1220059c82c19a53a1418fc376fc842fea834`. Expected answers come from that application's frozen executables, actual registered routes, generated schema, collected tests and synthetic database fixtures. Candidate implementations must never generate their own expected values.

## Existing evidence

- `tests/data/migration/inventory.json`: 86 route operations, 121 complete schemas, 13 full MCP tool schemas, 25 CLI help surfaces covering 18 public executable leaves, 20 runtime settings, 1,181 Rust test identities and platform/example-only source tests.
- `frontend-tests.json`: 294 collected assertions, source/report hashes, preservation and Go integration ownership.
- `portable-fixtures.json` and `rust/`: byte-preserved Rust observations and five synthetic SQLite databases. `.sqlite.fixture` avoids treating a fixture as live application storage. Keys and credentials are public test constants, never production material.
- `surfaces.json`: real loopback MCP initialization/tool listing, raw CLI outputs and a measured Windows debug resource baseline. This small workload does not establish release/container/helper performance.
- `baseline-evidence.json`: actual observed baseline check outcomes and hashes. A3 accepts fourteen unchanged pre-existing frontend formatting findings only. The remaining checks ran separately after the original all-mode command stopped at formatting.

The test-to-domain registry is deliberately conservative. Owners must inspect each assertion and refine cross-domain assignments before claiming that their phase covers it. Inventory counts, help output and representative fixtures are not full feature-parity evidence. Ignored helper entries, diagnostic benchmarks and Unix-only tests remain explicitly classified. Three example tests and the production static-export security test were executed separately.

## Commands

```text
node tests/migration/run.mjs --phase baseline --baseline 6bb1220059c82c19a53a1418fc376fc842fea834
node --test tests/migration/compare.test.mjs tests/migration/oracle.test.mjs tests/migration/exporter.test.mjs
```

The baseline runner validates archive identity, its entire file set, portable fixture hashes, test classifications, evidence identities, the narrow formatting exception and negative controls. Unknown or unfinished phases fail. They must acquire actual candidate drivers and proofs in their owning implementation tasks; no stub reports a pass. Normal verification never rebuilds or recaptures the oracle.

## Oracle preparation and recovery

`prepare-baseline.mjs --baseline <full-hash>` is an explicit one-time operation requiring the exact baseline HEAD and unchanged Rust/native inputs. It builds the application plus two test-only exporters, copies Cargo-reported executable paths and native Simple before export, captures toolchain/source/artifact identities, then publishes an archive under ignored `output/migration/oracle/<hash>/<platform>-<arch>`. `oracle-<platform>-<arch>.json` freezes its manifest hash. Preparation refuses existing archives or identities.

The current Windows archive's manifest hash is `dbf4f0495a8a3c1ae99d1ef939684838e63a48ec989d0ab3bfdb40e026fc6cfa`. Its three executable hashes were also checked against Cargo compiler-artifact paths. A transient Windows rename failure was recovered by revalidating every captured byte and moving the same prepared directory; the oracle was not regenerated. The earlier capture made before native-copy ordering was corrected is not accepted evidence.

If publication is interrupted, retain the `.preparing-*` directory and error log. Validate all files against its manifest, verify the approved source/build/native identities, check that the final archive and identity paths are absent, and complete publication of those exact bytes. Never overwrite a frozen identity or silently rebuild it. An archive alone is insufficient: verification requires its independently recorded identity. Platform-specific archives and ignored execution logs must be preserved externally as task evidence before deleting the workspace; committed portable fixtures remain available independently.

`capture-surfaces.mjs` explicitly captures additional frozen public surfaces from the verified archive into a new file. It seeds only a fresh marked OS-temporary root, listens only on loopback, uses synthetic credentials, never configures external notifications, and kills its owned service before cleanup. `collect-junit.py` and `inventory.mjs` are explicit baseline inventory producers; normal candidate checks read their frozen outputs.

## Comparison rules

JSON comparison preserves types, omission versus null, array order and identity; unsafe integer JSON must be compared as original bytes or decimal strings. Only registered scenario-specific wall-clock fields can be normalized, with an explicit reason. Durable scheduler creation times, slots, IDs, attempts and checkpoints are never covered by those exclusions. Persisted manifests and wire bytes use exact byte comparison. Ordered transition/effect traces reject replay, altered unknown states and duplicate side effects.

Random-nonce envelopes are frozen decryption/interoperability vectors; candidate encryption must preserve the algorithm, encoding and AAD contract, not reproduce a random nonce. The auth-v19 fixture reconstructs the actual historical scheduler table and preserves an unknown row plus an elevated AUTOINCREMENT sequence. It does not replace later all-status, empty-table, repeated-upgrade and conflict tests.
