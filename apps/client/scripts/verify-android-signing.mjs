/** Verify the APK is signed by the configured release keystore, rather than a debug key. */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import path from "node:path";
import { pathToFileURL } from "node:url";

/** Reject valid APK signatures whose signing certificate differs from the release key. */
export function assertAndroidSigner(output, expectedDigest) {
  assert.match(expectedDigest, /^[a-f0-9]{64}$/);
  const signers = [
    ...output.matchAll(
      /^Signer #\d+ certificate SHA-256 digest: ([a-fA-F0-9]{64})\s*$/gm,
    ),
  ].map((match) => match[1].toLowerCase());
  assert.deepEqual(
    signers,
    [expectedDigest],
    "APK must use the configured release signing certificate",
  );
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const certificate = execFileSync(
    "keytool",
    [
      "-exportcert",
      "-keystore",
      process.env.ANDROID_SIGNING_KEYSTORE,
      "-alias",
      process.env.ANDROID_SIGNING_KEY_ALIAS,
      "-storepass:env",
      "ANDROID_SIGNING_KEYSTORE_PASSWORD",
    ],
    { timeout: 60000 },
  );
  const output = execFileSync(
    path.join(process.env.ANDROID_HOME, "build-tools/36.0.0/apksigner"),
    [
      "verify",
      "--verbose",
      "--print-certs",
      "apps/client/bin/litradar-client-android-arm64.apk",
    ],
    { encoding: "utf8", timeout: 60000 },
  );
  assertAndroidSigner(
    output,
    createHash("sha256").update(certificate).digest("hex"),
  );
  console.log("Android release signature matches the configured keystore");
}
