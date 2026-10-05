import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { parsePublicKey, verifyMinisign } from "../lib/minisign.js";
import { PUBLIC_KEY_TEXT, RELEASE_BASE, downloadArtifact, resolveRelease } from "../lib/release.js";

const fixtures = join(dirname(fileURLToPath(import.meta.url)), "fixtures");
const read = (name) => readFileSync(join(fixtures, name));
const key = parsePublicKey(PUBLIC_KEY_TEXT);

test("a real release manifest verifies against the embedded key", () => {
  assert.equal(verifyMinisign(read("manifest.json"), read("manifest.json.minisig").toString(), key), true);
});

test("a real channel pointer verifies", () => {
  assert.equal(verifyMinisign(read("channel-stable.json"), read("channel-stable.json.minisig").toString(), key), true);
});

test("a tampered manifest does not verify", () => {
  const tampered = Buffer.from(read("manifest.json").toString().replace("0.9.1", "0.9.9"));
  assert.equal(verifyMinisign(tampered, read("manifest.json.minisig").toString(), key), false);
});

test("a different key does not verify", () => {
  const other = parsePublicKey(`untrusted comment: x\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3\n`);
  assert.equal(verifyMinisign(read("manifest.json"), read("manifest.json.minisig").toString(), other), false);
});

function fakeFetch(files) {
  return async (url) => {
    const bytes = files[url];
    if (!bytes) throw new Error(`unexpected fetch ${url}`);
    return Buffer.from(bytes);
  };
}

function releaseFiles(overrides = {}) {
  const repo = "CalvinMagezi/hq";
  const base = `${RELEASE_BASE}/${repo}/releases/download`;
  const manifest = read("manifest.json");
  const pointer = JSON.parse(read("channel-stable.json").toString());
  const manifestUrl = `${base}/v0.9.1-main.5/manifest.json`;
  return {
    repo,
    files: {
      [`${base}/channel-stable/channel-stable.json`]: read("channel-stable.json"),
      [`${base}/channel-stable/channel-stable.json.minisig`]: read("channel-stable.json.minisig"),
      [pointer.manifest_url]: manifest,
      [`${pointer.manifest_url}.minisig`]: read("manifest.json.minisig"),
      ...overrides,
    },
    manifestUrl,
  };
}

test("resolveRelease refuses a pointer whose manifest hash does not match", async () => {
  // main.5's pointer hash does not match main.6's manifest fixture, which is exactly the tampering case.
  const { repo, files } = releaseFiles();
  await assert.rejects(resolveRelease({ repo, channel: "stable", fetchFile: fakeFetch(files) }), /does not match the channel pointer|version differs/);
});

test("resolveRelease rejects bad repo and channel names", async () => {
  await assert.rejects(resolveRelease({ repo: "no-slash", channel: "stable" }), /repo must look like/);
  await assert.rejects(resolveRelease({ repo: "a/b", channel: "../x" }), /channel must be a plain name/);
});

test("downloadArtifact checks size and checksum", async () => {
  const body = Buffer.from("hello");
  const artifact = { name: "a", url: "u", size: body.length, sha256: createHash("sha256").update(body).digest("hex") };
  assert.deepEqual(await downloadArtifact(artifact, async () => body), body);
  await assert.rejects(downloadArtifact({ ...artifact, sha256: "0".repeat(64) }, async () => body), /checksum mismatch/);
  await assert.rejects(downloadArtifact({ ...artifact, size: 99 }, async () => body), /size mismatch/);
});

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, writeFileSync, readFileSync as readText } from "node:fs";
import { tmpdir } from "node:os";
import { extractBinary } from "../lib/install.js";

function tarball(files) {
  const dir = mkdtempSync(join(tmpdir(), "tb-"));
  for (const [name, body] of Object.entries(files)) writeFileSync(join(dir, name), body);
  const out = join(dir, "out.tar.gz");
  execFileSync("tar", ["-czf", out, "-C", dir, ...Object.keys(files)]);
  return readText(out);
}

test("extractBinary installs a single hq file and refuses archives with anything else", () => {
  const dest = mkdtempSync(join(tmpdir(), "dest-"));
  extractBinary(tarball({ hq: "#!/bin/sh\necho ok\n" }), { dir: join(dest, "bin"), file: join(dest, "bin", "hq") });
  assert.ok(existsSync(join(dest, "bin", "hq")));
  assert.throws(
    () => extractBinary(tarball({ hq: "x", extra: "y" }), { dir: dest, file: join(dest, "hq2") }),
    /exactly one file/,
  );
});

test("the embedded key matches release/minisign.pub in the repository", () => {
  const repoKey = readText(join(fixtures, "..", "..", "..", "..", "release", "minisign.pub"), "utf8");
  assert.deepEqual(parsePublicKey(repoKey), parsePublicKey(PUBLIC_KEY_TEXT));
});
