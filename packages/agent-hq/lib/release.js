import { createHash } from "node:crypto";
import { parsePublicKey, verifyMinisign } from "./minisign.js";

export const DEFAULT_REPO = "CalvinMagezi/hq";
export const RELEASE_BASE = "https://github.com";
const MAX_MANIFEST_BYTES = 1024 * 1024;

// Matches release/minisign.pub in the repository; a release signed by any other key is refused.
export const PUBLIC_KEY_TEXT = `untrusted comment: minisign public key 51FDC63C4F4E8BD2
RWTSi05PPMb9UVVnGilhLWT7h/mjQ1VjfAEXszxJB/Er8UEsCXFc3o1/
`;

const sha256 = (buffer) => createHash("sha256").update(buffer).digest("hex");

async function fetchBytes(url, limit) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) throw new Error(`GET ${url} failed: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length > limit) throw new Error(`${url} is larger than ${limit} bytes`);
  return bytes;
}

/** Fetches and fully verifies the channel pointer and manifest. Returns what to download. */
export async function resolveRelease({ repo, channel, fetchFile = fetchBytes, publicKeyText = PUBLIC_KEY_TEXT }) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo)) throw new Error("repo must look like owner/name");
  if (!/^[A-Za-z0-9_.-]+$/.test(channel)) throw new Error("channel must be a plain name");
  const key = parsePublicKey(publicKeyText);
  const base = `${RELEASE_BASE}/${repo}/releases/download`;
  const pointerUrl = `${base}/channel-${channel}/channel-${channel}.json`;

  const pointerBytes = await fetchFile(pointerUrl, MAX_MANIFEST_BYTES);
  const pointerSig = (await fetchFile(`${pointerUrl}.minisig`, MAX_MANIFEST_BYTES)).toString();
  if (!verifyMinisign(pointerBytes, pointerSig, key)) throw new Error("channel pointer signature does not verify");
  const pointer = JSON.parse(pointerBytes.toString());
  if (pointer.channel !== channel) throw new Error("channel pointer is for a different channel");
  if (typeof pointer.manifest_url !== "string" || !pointer.manifest_url.startsWith(`${base}/`)) {
    throw new Error("channel pointer points outside the release repository");
  }
  const tail = pointer.manifest_url.slice(base.length + 1);
  if (/(\.\.|\/\/|\\|\?|#|@|%2[eEfF]|%5[cC]|%00)/.test(tail)) throw new Error("channel pointer URL looks unsafe");

  const manifestBytes = await fetchFile(pointer.manifest_url, MAX_MANIFEST_BYTES);
  if (sha256(manifestBytes) !== pointer.manifest_sha256) throw new Error("manifest does not match the channel pointer");
  const manifestSig = (await fetchFile(`${pointer.manifest_url}.minisig`, MAX_MANIFEST_BYTES)).toString();
  if (!verifyMinisign(manifestBytes, manifestSig, key)) throw new Error("manifest signature does not verify");
  const manifest = JSON.parse(manifestBytes.toString());
  if (manifest.version !== pointer.version) throw new Error("manifest version differs from the pointer");
  if (!/^[A-Za-z0-9._+-]+$/.test(manifest.version)) throw new Error("unexpected version string");

  const name = `hq-${manifest.version}-linux-x86_64.tar.gz`;
  const artifact = manifest.artifacts?.find((a) => a.name === name);
  return {
    version: manifest.version,
    gitSha: manifest.git_sha,
    artifact: artifact && {
      name,
      sha256: artifact.sha256,
      size: artifact.size,
      url: `${pointer.manifest_url.slice(0, pointer.manifest_url.lastIndexOf("/"))}/${name}`,
    },
  };
}

export async function downloadArtifact(artifact, fetchFile = fetchBytes) {
  const bytes = await fetchFile(artifact.url, artifact.size + 1);
  if (bytes.length !== artifact.size) throw new Error(`size mismatch for ${artifact.name}`);
  if (sha256(bytes) !== artifact.sha256) throw new Error(`checksum mismatch for ${artifact.name}`);
  return bytes;
}
