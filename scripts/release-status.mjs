/** Fail the workflow when any required release stage was skipped or unsuccessful. */
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";

/** Require publication and promotion, allowing only an already published retry to skip builds. */
export function verifyReleaseJobs(jobs) {
  assert.equal(jobs.context?.result, "success", "Release context must succeed");
  const published = jobs.context.outputs?.published;
  assert(
    ["true", "false"].includes(published),
    "Missing release publication state",
  );
  const required =
    published === "true"
      ? ["promote"]
      : ["windows", "linux", "publish", "promote"];
  for (const job of required) {
    assert.equal(
      jobs[job]?.result,
      "success",
      `Required release job ${job} did not succeed`,
    );
  }
  if (published === "true") {
    for (const job of ["windows", "linux", "publish"]) {
      assert.equal(
        jobs[job]?.result,
        "skipped",
        `Published release must not rebuild ${job}`,
      );
    }
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  verifyReleaseJobs(JSON.parse(process.env.RELEASE_JOB_RESULTS));
}
