/** Negative controls prevent filtered or empty Go runs from satisfying primitive gates. */
import assert from "node:assert/strict";
import test from "node:test";
import { command } from "./run.mjs";

test("inherited Go filters and workspace configuration cannot alter proof selection", async () => {
  const original = {
    GOFLAGS: process.env.GOFLAGS,
    GOWORK: process.env.GOWORK,
    GOENV: process.env.GOENV,
  };
  Object.assign(process.env, {
    GOFLAGS: "-run=^$",
    GOWORK: "unrelated.work",
    GOENV: "unrelated.env",
  });
  try {
    await command("environment-negative-control", process.execPath, [
      "-e",
      "const a=require('node:assert/strict'); a.equal(process.env.GOFLAGS,''); a.equal(process.env.GOWORK,'off'); a.equal(process.env.GOENV,'off');",
    ]);
  } finally {
    for (const [key, value] of Object.entries(original)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
});

test("successful process with no executed Go tests cannot pass", async () => {
  await assert.rejects(
    command("empty-negative-regular", process.execPath, [
      "-e",
      "console.log(JSON.stringify({Action:'pass',Package:'fixture'}))",
    ]),
    /no tests executed/,
  );
});

test("a timed out proof returns failure after terminating its process tree", async () => {
  await assert.rejects(
    command(
      "timeout-negative-control",
      process.execPath,
      ["-e", "setInterval(()=>{},1000)"],
      150,
    ),
    /Deadline exceeded/,
  );
});
