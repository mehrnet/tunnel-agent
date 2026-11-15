import type { ClusterControlKeyRow, Env } from "./models";
import { base64Standard, base64Url } from "./encoding";
import { now } from "./validation";

const SSH_ED25519 = new TextEncoder().encode("ssh-ed25519");

export async function clusterControlPublicKey(env: Env, clusterId: string): Promise<string> {
  const existing = await env.DB.prepare(
    "SELECT ssh_public_key FROM cluster_control_keys WHERE cluster_id = ?",
  ).bind(clusterId).first<ClusterControlKeyRow>();
  if (existing) return existing.ssh_public_key;

  const created = await createControlKey(env, clusterId);
  await env.DB.prepare(
    `INSERT OR IGNORE INTO cluster_control_keys
      (cluster_id, ssh_public_key, encrypted_private_key, created_at) VALUES (?, ?, ?, ?)`,
  ).bind(clusterId, created.publicKey, created.encryptedPrivateKey, now()).run();
  const stored = await env.DB.prepare(
    "SELECT ssh_public_key FROM cluster_control_keys WHERE cluster_id = ?",
  ).bind(clusterId).first<ClusterControlKeyRow>();
  if (!stored) throw new Error("could not persist cluster SSH control key");
  return stored.ssh_public_key;
}

export async function clusterControlPrivateKey(env: Env, clusterId: string): Promise<string> {
  const stored = await env.DB.prepare(
    "SELECT encrypted_private_key FROM cluster_control_keys WHERE cluster_id = ?",
  ).bind(clusterId).first<{ encrypted_private_key: string }>();
  if (!stored) throw new Error("cluster control key is not initialized");
  const privateKey = await decryptPrivateKey(env, clusterId, stored.encrypted_private_key);
  return pem("PRIVATE KEY", privateKey);
}

async function createControlKey(env: Env, clusterId: string): Promise<{ publicKey: string; encryptedPrivateKey: string }> {
  const pair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]) as CryptoKeyPair;
  const publicRaw = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey) as ArrayBuffer);
  const privatePkcs8 = new Uint8Array(await crypto.subtle.exportKey("pkcs8", pair.privateKey) as ArrayBuffer);
  return {
    publicKey: sshPublicKey(publicRaw),
    encryptedPrivateKey: await encryptPrivateKey(env, clusterId, privatePkcs8),
  };
}

async function encryptPrivateKey(env: Env, clusterId: string, privateKey: Uint8Array): Promise<string> {
  const secret = base64Standard(env.CLUSTER_CONTROL_KEY_ENCRYPTION_KEY ?? "");
  if (!secret || secret.length !== 32) throw new Error("cluster control key encryption is not configured");
  const key = await crypto.subtle.importKey("raw", secret, "AES-GCM", false, ["encrypt"]);
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const cipher = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv, additionalData: new TextEncoder().encode(clusterId) }, key, privateKey,
  );
  return `${base64Url(iv)}.${base64Url(cipher)}`;
}

async function decryptPrivateKey(env: Env, clusterId: string, encrypted: string): Promise<Uint8Array> {
  const [encodedIv, encodedCipher] = encrypted.split(".");
  const secret = base64Standard(env.CLUSTER_CONTROL_KEY_ENCRYPTION_KEY ?? "");
  const iv = encodedIv ? base64UrlBytes(encodedIv) : null;
  const cipher = encodedCipher ? base64UrlBytes(encodedCipher) : null;
  if (!secret || secret.length !== 32 || !iv || iv.length !== 12 || !cipher) {
    throw new Error("cluster control key is unreadable");
  }
  const key = await crypto.subtle.importKey("raw", secret, "AES-GCM", false, ["decrypt"]);
  try {
    return new Uint8Array(await crypto.subtle.decrypt(
      { name: "AES-GCM", iv, additionalData: new TextEncoder().encode(clusterId) }, key, cipher,
    ));
  } catch {
    throw new Error("cluster control key cannot be decrypted");
  }
}

function base64UrlBytes(value: string): Uint8Array | null {
  const padded = value.replaceAll("-", "+").replaceAll("_", "/") + "=".repeat((4 - value.length % 4) % 4);
  return base64Standard(padded);
}

function pem(label: string, bytes: Uint8Array): string {
  const text = btoa(String.fromCharCode(...bytes));
  const lines = text.match(/.{1,64}/g)?.join("\n") ?? "";
  return `-----BEGIN ${label}-----\n${lines}\n-----END ${label}-----\n`;
}

function sshPublicKey(raw: Uint8Array): string {
  if (raw.length !== 32) throw new Error("generated SSH control key has invalid length");
  const blob = new Uint8Array(4 + SSH_ED25519.length + 4 + raw.length);
  const view = new DataView(blob.buffer);
  view.setUint32(0, SSH_ED25519.length);
  blob.set(SSH_ED25519, 4);
  view.setUint32(4 + SSH_ED25519.length, raw.length);
  blob.set(raw, 8 + SSH_ED25519.length);
  let binary = "";
  for (const byte of blob) binary += String.fromCharCode(byte);
  return btoa(binary);
}
