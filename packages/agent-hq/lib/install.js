import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { DEFAULT_REPO, downloadArtifact, hasPrebuiltFor, platformKey, resolveRelease } from "./release.js";

// Platforms without a published binary (for example Intel Macs) build from source.
export const hasPrebuilt = (platform = platformKey()) => hasPrebuiltFor(platform);

export function defaultPrefix() {
  return join(homedir(), ".local", "bin");
}

const VERSION_CHECK_TIMEOUT_MS = 15_000;

// Runs the staged binary before it can replace a working install. A wrong-libc or wrong-CPU
// build fails here instead of leaving the user without a working hq.
function checkStaged(staged, expectVersion) {
  let reported;
  try {
    reported = execFileSync(staged, ["--version"], { encoding: "utf8", timeout: VERSION_CHECK_TIMEOUT_MS, stdio: ["ignore", "pipe", "pipe"] });
  } catch (error) {
    rmSync(staged, { force: true });
    throw new Error(`the downloaded hq does not run on this machine (${error.message.split("\n")[0]}). It may need a newer glibc than this system has, or this may be a musl system such as Alpine. Your existing install was left unchanged.`);
  }
  if (!reported.includes(expectVersion)) {
    rmSync(staged, { force: true });
    throw new Error(`the downloaded hq reports "${reported.trim()}" instead of version ${expectVersion}. Your existing install was left unchanged.`);
  }
  return reported.trim();
}

export function extractBinary(tarball, destination, { expectVersion } = {}) {
  const work = mkdtempSync(join(tmpdir(), "agent-hq-"));
  try {
    const archive = join(work, "hq.tar.gz");
    writeFileSync(archive, tarball);
    const listing = execFileSync("tar", ["-tzf", archive], { encoding: "utf8" }).trim().replace(/^\.\//gm, "");
    if (listing !== "hq") throw new Error("release archive must contain exactly one file named hq");
    execFileSync("tar", ["-xzf", archive, "-C", work, "--no-same-owner", "hq"]);
    mkdirSync(destination.dir, { recursive: true });
    const staged = join(destination.dir, ".hq.new");
    copyFileSync(join(work, "hq"), staged);
    chmodSync(staged, 0o755);
    if (expectVersion) checkStaged(staged, expectVersion);
    renameSync(staged, destination.file);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

export async function installPrebuilt({ repo = DEFAULT_REPO, channel = "stable", prefix = defaultPrefix(), platform = platformKey(), log = console.log } = {}) {
  log(`Looking up the ${channel} release of ${repo}...`);
  const release = await resolveRelease({ repo, channel, platform });
  if (!release.artifact) {
    throw new Error(`release ${release.version} has no ${platform} build yet; try --channel main or --from-source`);
  }
  log(`Verified signature for ${release.version} (${String(release.gitSha).slice(0, 7)}). Downloading...`);
  const tarball = await downloadArtifact(release.artifact);
  const destination = { dir: prefix, file: join(prefix, "hq") };
  extractBinary(tarball, destination, { expectVersion: release.version });
  const reported = execFileSync(destination.file, ["--version"], { encoding: "utf8" }).trim();
  log(`Installed ${destination.file}: ${reported}`);
  return { ...release, path: destination.file };
}

export function installFromSource({ repo = DEFAULT_REPO, version, log = console.log } = {}) {
  const cargo = spawnSync("cargo", ["--version"], { encoding: "utf8" });
  if (cargo.status !== 0) {
    throw new Error("No prebuilt binary for this platform and Rust is not installed. Install Rust from https://rustup.rs, then run this again.");
  }
  const args = ["install", "--locked", "--git", `https://github.com/${repo}.git`, "hq-cli"];
  if (version) args.push("--tag", `v${version}`);
  log(`Building from source with: cargo ${args.join(" ")}`);
  const result = spawnSync("cargo", args, { stdio: "inherit" });
  if (result.status !== 0) throw new Error("cargo install failed");
}

export function pathAdvice(prefix) {
  const onPath = (process.env.PATH ?? "").split(delimiter).includes(prefix);
  return onPath ? null : `Add ${prefix} to your PATH, for example: export PATH="${prefix}:$PATH"`;
}
