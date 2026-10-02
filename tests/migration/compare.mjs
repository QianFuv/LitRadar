/**
 * Compare independently captured migration observations without hiding identity changes.
 */
import assert from "node:assert/strict";

const SCENARIO_NORMALIZATIONS = new Map([
  ["login-wall-clock-expiry", new Set(["/expires_at"])],
  ["masked-settings-wall-clock", new Set(["/created_at", "/updated_at"])],
]);

/**
 * Reject decoded values whose precision or representation has already been lost.
 * @param {unknown} value - Parsed JSON observation.
 * @returns {void}
 */
function validateJson(value) {
  if (typeof value === "number") {
    assert(Number.isFinite(value), "Non-finite JSON number");
    assert(
      !Number.isInteger(value) || Number.isSafeInteger(value),
      "Unsafe JSON integer: compare original bytes or decimal strings",
    );
  } else if (Array.isArray(value)) {
    value.forEach(validateJson);
  } else if (value !== null && typeof value === "object") {
    assert.equal(
      Object.getPrototypeOf(value),
      Object.prototype,
      "Expected plain JSON object",
    );
    Object.values(value).forEach(validateJson);
  } else {
    assert(
      value === null || ["string", "boolean"].includes(typeof value),
      "Invalid JSON value",
    );
  }
}

/**
 * Compare JSON structure, retaining omission, types and ordered arrays.
 * @param {unknown} expected - Frozen Rust observation.
 * @param {unknown} actual - Candidate observation.
 * @param {{pointer: string, reason: string}[]} normalizations - Explicit volatile root timestamps only.
 * @param {string} scenario - Frozen scenario authorizing those exclusions.
 * @returns {void}
 */
export function compareJson(
  expected,
  actual,
  normalizations = [],
  scenario = "",
) {
  validateJson(expected);
  validateJson(actual);
  const expectedCopy = structuredClone(expected);
  const actualCopy = structuredClone(actual);
  const seen = new Set();
  for (const { pointer, reason } of normalizations) {
    assert(
      SCENARIO_NORMALIZATIONS.get(scenario)?.has(pointer),
      `Forbidden normalization for ${scenario}: ${pointer}`,
    );
    assert(
      typeof reason === "string" && reason.trim(),
      "Normalization requires a reason",
    );
    assert(!seen.has(pointer), "Duplicate normalization");
    seen.add(pointer);
    const field = pointer.slice(1);
    for (const value of [expectedCopy, actualCopy]) {
      assert(
        value && !Array.isArray(value) && Object.hasOwn(value, field),
        `Missing normalized field: ${pointer}`,
      );
    }
    assert.equal(
      typeof actualCopy[field],
      typeof expectedCopy[field],
      "Timestamp type changed",
    );
    assert(
      ["number", "string"].includes(typeof expectedCopy[field]),
      "Invalid timestamp value",
    );
    expectedCopy[field] = actualCopy[field] = "<explicit volatile timestamp>";
  }
  assert.deepStrictEqual(actualCopy, expectedCopy);
}

/**
 * Compare byte-sensitive persisted formats without decoding or newline normalization.
 * @param {Uint8Array} expected - Frozen bytes.
 * @param {Uint8Array} actual - Candidate bytes.
 * @returns {void}
 */
export function compareBytes(expected, actual) {
  assert(
    expected instanceof Uint8Array && actual instanceof Uint8Array,
    "Expected byte buffers",
  );
  assert.deepStrictEqual(Buffer.from(actual), Buffer.from(expected));
}

/**
 * Compare ordered state transitions and synthetic external effects without exclusions.
 * @param {unknown[]} expected - Frozen trace with identities and side-effect counts.
 * @param {unknown[]} actual - Candidate trace.
 * @returns {void}
 */
export function compareTrace(expected, actual) {
  assert(
    Array.isArray(expected) && Array.isArray(actual),
    "Expected ordered traces",
  );
  compareJson(expected, actual);
}
