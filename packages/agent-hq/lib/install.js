import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { homedir, platform, arch, tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { DEFAULT_REPO, downloadArtifact, resolveRelease } from "./release.js";

export const hasPrebuilt = () => platform() === "linux" && arch() === "x64";

export function defaultPrefix() {
  return join(homedir(), ".local", "bin");
}

export function extractBinary(tarball, destination) {
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
    renameSync(staged, destination.file);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

export async function installPrebuilt({ repo = DEFAULT_REPO, channel = "stable", prefix = defaultPrefix(), log = console.log } = {}) {
  log(`Looking up the ${channel} release of ${repo}...`);
  const release = await resolveRelease({ repo, channel });
  if (!release.artifact) throw new Error(`release ${release.version} has no linux-x86_64 build`);
  log(`Verified signature for ${release.version} (${String(release.gitSha).slice(0, 7)}). Downloading...`);
  const tarball = await downloadArtifact(release.artifact);
  const destination = { dir: prefix, file: join(prefix, "hq") };
  extractBinary(tarball, destination);
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
