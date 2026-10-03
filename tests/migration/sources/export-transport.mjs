/** Freeze independent source transport observations from the compiled original Rust helper. */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { digest } from "../oracle.mjs";

const requests = [];
const now = "2015-10-21T07:20:00Z";
requests.push(
  {
    kind: "retry",
    now: "2024-02-29T08:49:37Z",
    input: "Friday, 01-Mar-74 00:00:00 GMT",
  },
  {
    kind: "retry",
    now: "2026-11-06T08:49:59.500Z",
    input: "Saturday, 06-Nov-76 08:49:60 GMT",
  },
);
for (const input of [
  "TueJan2017:35:202015",
  "Tue Jan 20 17: 35: 20 2015",
  "Tue Jan 20 17:35:20 00002015",
  "Tuesday,\t20-Jan-15 17:35:20 GMT",
  "Thursday, 01-Mar-74 08:49:37 GMT",
  "Tuesday, 20-Jan-15 17: 35: 20GMT",
])
  requests.push({ kind: "retry", now: "2024-02-29T08:49:37Z", input });
for (const input of [
  "http://u%zz:p@127.0.0.1:8080",
  "http://u%FF:p@127.0.0.1:8080",
  "http://:@localhost",
  "http://@localhost",
  "http://user:@localhost",
  "http://user:pass@localhost",
  "HTTPS://localhost/",
  "http://127.1",
  "http:localhost",
  "socks5h://user:pass@localhost",
  "socks5h://localhost?",
  "http://localhost#",
  "http://localhost/path",
  "socks4://localhost",
])
  requests.push({ kind: "proxy", input });
for (const input of [
  "",
  "0",
  " 12 ",
  "-1",
  "+1",
  "0.5",
  "18446744073709551615",
  "18446744073709551616",
  "Wed, 21 Oct 2015 07:28:00 GMT",
  "Wed, 21 Oct 2015 07:00:00 GMT",
  "Thu, 21 Oct 2015 07:28:00 GMT",
  "Wed , 21 Oct 2015 07:28:00 GMT",
  "wed, 21 oct 2015 07:28:00 gmt",
  "21 Oct 2015 07:28 GMT",
  "21 Oct 2015 07 : 28 :00 GMT",
  "21 Oct 2015 07:28: 00 GMT",
  "21 Oct 2015 7:28:00 GMT",
  "21 Oct 2015 07:8:00 GMT",
  "21 Oct 2015 07:28:0 GMT",
  "21 Oct 2015 07:28:60 GMT",
  "21 Oct 2015 07:28:61 GMT",
  "21 Oct 2015 24:28:00 GMT",
  "31 Feb 2015 07:28:00 GMT",
  "21 Oct 2015 07:28:00 GMT (ok (nested)) (again)",
  "21 Oct 2015 07:28:00 GMT (ok\\))",
  "21 Oct 2015 07:28:00 GMT (unclosed",
  "21 Oct 2015 07:28:00 GMT junk",
  "21\u00a0Oct\u20032015\t07:28:00 GMT",
  "21 Oct (comment) 2015 07:28:00 GMT",
  "Wed Oct 21 07:28:00 2015",
  "Thu Oct 21 07:28:00 2015",
  "Wednesday, 21-Oct-15 07:28:00 GMT",
  "wednesday, 21-Oct-15 07:28:00 GMT",
  "Wed, 21-Oct-15 07:28:00 GMT",
])
  requests.push({ kind: "retry", now, input });
for (const year of [
  "00",
  "49",
  "50",
  "68",
  "69",
  "99",
  "009",
  "112",
  "999",
  "0049",
  "0000",
  "262142",
  "262143",
  "999999999999999999999",
])
  requests.push({ kind: "retry", now, input: `21 Oct ${year} 07:28:00 GMT` });
for (const zone of [
  "GMT",
  "UT",
  "UTC",
  "Zulu",
  "EST",
  "EDT",
  "CST",
  "CDT",
  "MST",
  "MDT",
  "PST",
  "PDT",
  "+0000",
  "-0000",
  "+2359",
  "-2359",
  "+2400",
  "+0060",
  "+01:00",
  ..."ABCDEFGHIJKLMNOPQRSTUVWXYZ",
])
  requests.push({ kind: "retry", now, input: `21 Oct 2015 07:28:00 ${zone}` });
for (const now of [
  "1994-11-06T08:00:00Z",
  "2026-01-01T00:00:00Z",
  "2026-11-06T08:49:36Z",
  "2026-11-06T08:49:37Z",
  "2026-11-06T08:49:38Z",
  "2024-02-29T08:49:37Z",
])
  for (const input of [
    "Sunday, 06-Nov-94 08:49:37 GMT",
    "Sun Nov  6 08:49:37 1994",
    "Wednesday, 06-Nov-75 08:49:37 GMT",
    "Sunday, 06-Nov-77 08:49:37 GMT",
    "Friday, 06-Nov-76 08:49:37 GMT",
  ])
    requests.push({ kind: "retry", now, input });
for (const now of [
  "2026-01-01T11:00:00Z",
  "2026-01-01T13:00:00Z",
  "2026-01-02T12:00:59Z",
  "2026-01-02T12:00:59.5Z",
  "2026-01-02T12:01:00Z",
])
  for (const input of [
    "02 Jan 2026 12:00:60 GMT",
    "02 Jan 2026 13:00:60 +0100",
    "02 Jan 2026 07:00:60 EST",
  ])
    requests.push({ kind: "retry", now, input });
for (const bytes of [
  [],
  [255, 255],
  [192, 128],
  [230, 131],
  [230, 131, 65],
  [241, 128, 128],
  [237, 160, 128],
  [240, 128, 128, 128],
  [239, 191, 189],
  [239, 187, 191, 65],
])
  requests.push({ kind: "text", bytes });
for (let first = 0; first < 256; first++)
  for (const suffix of [
    [],
    [128],
    [128, 128],
    [160, 128],
    [191, 191, 191],
    [65],
    [128, 65],
  ])
    requests.push({ kind: "text", bytes: [first, ...suffix] });
for (const input of [
  "null",
  "true",
  '{"id":18446744073709551615}',
  "-0",
  "1e400",
  "1e-400",
  '"\\ud800"',
  '"\\udc00"',
  '"\\ud800\\udc00"',
  '{"x":"\\ud800","x":1}',
  "1 2",
  "[".repeat(127) + "0" + "]".repeat(127),
  "[".repeat(128) + "0" + "]".repeat(128),
])
  requests.push({ kind: "json", input });
const binary = "output/migration/execution/sources-transport-oracle.exe";
const result = spawnSync(binary, [], {
  input: requests.map((item) => JSON.stringify(item)).join("\n") + "\n",
  encoding: "utf8",
  windowsHide: true,
  timeout: 60000,
  maxBuffer: 16 * 1024 * 1024,
});
assert.ifError(result.error);
assert.equal(result.status, 0, result.stderr);
const observations = result.stdout
  .trim()
  .split(/\r?\n/)
  .map((line) => JSON.parse(line));
assert.equal(observations.length, requests.length);
await fs.writeFile(
  "tests/migration/sources/transport-vectors.json",
  JSON.stringify(
    {
      provenance: JSON.parse(
        await fs.readFile(
          "output/migration/execution/t05-transport-oracle-build.json",
          "utf8",
        ),
      ),
      exporter_sha256: digest(
        await fs.readFile("tests/migration/sources/export-transport.mjs"),
      ),
      observations,
    },
    null,
    2,
  ) + "\n",
);
console.log(
  `Frozen ${observations.length} original Rust transport observations`,
);
