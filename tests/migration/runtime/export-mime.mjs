/** Preserve the locked Rust static-server MIME data without a host registry dependency. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { digest } from "../oracle.mjs";

const directory = process.argv[2];
assert(directory, "provide the local mime_guess-2.0.5 crate directory");
const manifest = await fs.readFile(path.join(directory, "Cargo.toml"), "utf8");
assert.match(manifest, /name = "mime_guess"/);
assert.match(manifest, /version = "2\.0\.5"/);
const source = await fs.readFile(path.join(directory, "src/mime_types.rs"));
const table = source.toString("utf8").split("pub static MIME_TYPES:")[1];
assert(table, "locked MIME table declaration is missing");
const entries = [...table.matchAll(/\(\s*"([^"]+)",\s*&\[\s*"([^"]+)"/g)];
assert.equal(entries.length, [...table.matchAll(/\(\s*"([^"]+)",/g)].length);
assert(entries.length > 1000);
const output = "internal/runtime/mime";
await fs.mkdir(output, { recursive: true });
const data = Buffer.from(
  entries.map((entry) => `${entry[1]}\t${entry[2]}\n`).join(""),
);
await fs.writeFile(path.join(output, "types.tsv"), data);
await fs.copyFile(
  path.join(directory, "LICENSE"),
  path.join(output, "LICENSE"),
);
await fs.writeFile(
  path.join(output, "README.md"),
  `# Static MIME compatibility data\n\nFirst MIME type for each extension from the original locked mime_guess 2.0.5 table. The application embeds this data so Windows registry changes cannot alter public responses. Upstream license is retained in LICENSE.\n\n- Source: https://github.com/abonander/mime_guess/tree/2.0.5\n- Original src/mime_types.rs SHA-256: ${digest(source)}\n- Generated types.tsv SHA-256: ${digest(data)}\n- Entries: ${entries.length}\n- Regenerate: node tests/migration/runtime/export-mime.mjs PATH_TO_MIME_GUESS_2.0.5\n\nThis is compatibility data, not a new dependency. Updating it requires an intentional public MIME compatibility decision.\n`,
);
console.log(
  JSON.stringify({ mime_entries: entries.length, sha256: digest(data) }),
);
