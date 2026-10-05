import { createHash, createPublicKey, verify } from "node:crypto";

const SPKI_ED25519_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const KEY_ID_LENGTH = 8;
const SIGNATURE_LENGTH = 64;

function secondLine(text, what) {
  const lines = text.split(/\r?\n/);
  if (lines.length < 2 || !lines[1]) throw new Error(`${what} is not a minisign file`);
  return lines;
}

export function parsePublicKey(text) {
  const raw = Buffer.from(secondLine(text, "public key")[1], "base64");
  if (raw.length !== 2 + KEY_ID_LENGTH + 32 || raw.toString("latin1", 0, 2) !== "Ed") {
    throw new Error("unsupported public key");
  }
  return { keyId: raw.subarray(2, 2 + KEY_ID_LENGTH), key: raw.subarray(2 + KEY_ID_LENGTH) };
}

function ed25519Verify(publicKey, data, signature) {
  const key = createPublicKey({
    key: Buffer.concat([SPKI_ED25519_PREFIX, publicKey.key]),
    format: "der",
    type: "spki",
  });
  return verify(null, data, key, signature);
}

/** Verifies a minisign signature file (both the file signature and the trusted comment). */
export function verifyMinisign(message, signatureText, publicKey) {
  const lines = secondLine(signatureText, "signature");
  const raw = Buffer.from(lines[1], "base64");
  if (raw.length !== 2 + KEY_ID_LENGTH + SIGNATURE_LENGTH) return false;
  const algorithm = raw.toString("latin1", 0, 2);
  if (!raw.subarray(2, 2 + KEY_ID_LENGTH).equals(publicKey.keyId)) return false;
  const signature = raw.subarray(2 + KEY_ID_LENGTH);

  let signed;
  if (algorithm === "ED") signed = createHash("blake2b512").update(message).digest();
  else if (algorithm === "Ed") signed = message;
  else return false;
  if (!ed25519Verify(publicKey, signed, signature)) return false;

  const trusted = (lines[2] ?? "").replace(/^trusted comment: ?/, "");
  const globalSignature = Buffer.from(lines[3] ?? "", "base64");
  if (globalSignature.length !== SIGNATURE_LENGTH) return false;
  return ed25519Verify(publicKey, Buffer.concat([signature, Buffer.from(trusted)]), globalSignature);
}
