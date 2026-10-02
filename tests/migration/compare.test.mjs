/**
 * Negative controls ensure parity comparators cannot erase consequential differences.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { compareBytes, compareJson, compareTrace } from "./compare.mjs";

test("JSON object key order is irrelevant while array order and identity remain exact", () => {
  compareJson(
    { article_id: "9007199254740993", items: [1, 2] },
    { items: [1, 2], article_id: "9007199254740993" },
  );
  for (const changed of [
    { article_id: "9007199254740992", items: [1, 2] },
    { article_id: "9007199254740993", items: [2, 1] },
    { article_id: "9007199254740993", items: [1, 2], secret: "unexpected" },
  ]) {
    assert.throws(() =>
      compareJson({ article_id: "9007199254740993", items: [1, 2] }, changed),
    );
  }
});

test("missing, null, empty and numeric/string values are distinct", () => {
  for (const changed of [{}, { value: null }, { value: [] }, { value: 0 }]) {
    assert.throws(() => compareJson({ value: "" }, changed));
  }
  assert.throws(() =>
    compareJson({ id: 9007199254740992 }, { id: 9007199254740992 }),
  );
});

test("only explicitly justified existing volatile fields can be normalized", () => {
  const rule = [
    {
      pointer: "/expires_at",
      reason: "Existing login scenario freezes wall-clock expiry",
    },
  ];
  compareJson(
    { expires_at: 100, user_id: "7" },
    { expires_at: 200, user_id: "7" },
    rule,
    "login-wall-clock-expiry",
  );
  assert.throws(() =>
    compareJson(
      { created_at: 100 },
      { created_at: 200 },
      [
        {
          pointer: "/created_at",
          reason: "Durable creation identity must remain exact",
        },
      ],
      "scheduler-state",
    ),
  );
  assert.throws(() =>
    compareJson({ expires_at: 100 }, {}, rule, "login-wall-clock-expiry"),
  );
  assert.throws(() =>
    compareJson(
      { expires_at: 100 },
      { expires_at: "100" },
      rule,
      "login-wall-clock-expiry",
    ),
  );
  for (const pointer of [
    "/user_id",
    "/slot",
    "/checkpoint",
    "/attempt_id",
    "/fingerprint",
    "/items",
  ]) {
    assert.throws(() =>
      compareJson({}, {}, [{ pointer, reason: "Do not hide identity" }]),
    );
  }
  assert.throws(() =>
    compareJson({ expires_at: 1 }, { expires_at: 2 }, [
      { pointer: "/expires_at", reason: "" },
    ]),
  );
});

test("byte comparison rejects changed newline, encoding and payload", () => {
  const expected = Buffer.from("manifest:v1\n");
  compareBytes(expected, Buffer.from(expected));
  for (const changed of ["manifest:v1", "manifest:v1\r\n", "manifest:v2\n"]) {
    assert.throws(() => compareBytes(expected, Buffer.from(changed)));
  }
});

test("trace comparison catches replay, state, sequence and effect-count changes", () => {
  const expected = [
    { state: "sending", sequence: "9", effects: 1 },
    { state: "unknown", sequence: "9", effects: 1 },
  ];
  compareTrace(expected, structuredClone(expected));
  const mutations = [
    [
      { state: "sending", sequence: "9", effects: 1 },
      { state: "unknown", sequence: "9", effects: 2 },
    ],
    [
      { state: "sending", sequence: "9", effects: 1 },
      { state: "failed", sequence: "9", effects: 1 },
    ],
    [
      { state: "sending", sequence: "10", effects: 1 },
      { state: "unknown", sequence: "10", effects: 1 },
    ],
    [...expected].reverse(),
    [...expected, expected[0]],
  ];
  for (const changed of mutations)
    assert.throws(() => compareTrace(expected, changed));
});
