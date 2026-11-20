import type { DocumentRow, Env, NodeRow, OwnerRow } from "./models";
import { audit, nodeDocument } from "./store";
import { BUNDLE_ROOT, now } from "./validation";

export const MAX_MANIFEST_BYTES = 256 * 1024;

const ROOT_FIELDS = new Set(["version", "node_scope", "bundle", "agent", "tunnels"]);
const AGENT_FIELDS = new Set(["poll_interval_secs", "max_active_lanes", "lanes", "bundle_seed"]);
const CONTRACTS = new Set([
  "hpnssh.forward.v1",
  "hpnssh.server.v1",
  "wstunnel.server.v1",
  "wstunnel.reverse.v1",
  "rathole.service.v1",
  "trusttunnel.endpoint.v1",
]);
const PROTECTED_FIELDS = new Set([
  "access_key",
  "api_key",
  "authorization",
  "credential",
  "credentials",
  "passphrase",
  "password",
  "private_key",
  "privatekey",
  "secret",
  "token",
]);

export async function replaceNodeDocument(
  env: Env,
  owner: OwnerRow,
  node: NodeRow,
  rawManifest: string,
): Promise<DocumentRow> {
  const manifestJson = normalizeManifest(rawManifest, node);
  const current = await nodeDocument(env, node.id);
  if (!current) throw new Error("server has no desired document");

  const revision = current.revision + 1;
  const updatedAt = now();
  const result = await env.DB.prepare(
    `UPDATE node_documents
        SET revision = ?, manifest_json = ?, updated_at = ?
      WHERE node_id = ? AND revision = ?
        AND NOT EXISTS (SELECT 1 FROM tunnel_intents t WHERE t.path_key IS NOT NULL
                        AND node_documents.node_id IN (t.source_node_id,t.listener_node_id))`,
  ).bind(revision, manifestJson, updatedAt, node.id, current.revision).run();
  if (result.meta.changes !== 1) throw new Error("server document changed concurrently or is managed by the lane editor");

  await audit(env, "node.document_updated", {
    bytes: utf8Bytes(manifestJson),
    revision,
    version: 3,
  }, { ownerId: owner.id, nodeId: node.id });
  return { revision, manifest_json: manifestJson, updated_at: updatedAt };
}

function normalizeManifest(rawManifest: string, node: NodeRow): string {
  if (utf8Bytes(rawManifest) > MAX_MANIFEST_BYTES) {
    throw new Error(`manifest exceeds ${MAX_MANIFEST_BYTES} bytes`);
  }
  let value: unknown;
  try {
    value = JSON.parse(rawManifest);
  } catch {
    throw new Error("manifest is not valid JSON");
  }
  if (!isRecord(value)) throw new Error("manifest must be a JSON object");
  validateRoot(value, node);
  rejectProtectedValues(value, "manifest");
  const normalized = JSON.stringify(value);
  if (utf8Bytes(normalized) > MAX_MANIFEST_BYTES) throw new Error("normalized manifest is too large");
  return normalized;
}

function validateRoot(value: Record<string, unknown>, node: NodeRow): void {
  for (const field of Object.keys(value)) {
    if (!ROOT_FIELDS.has(field)) throw new Error(`manifest field ${field} is not allowed for version 3`);
  }
  if (value.version !== 3) throw new Error("manifest version must be 3");
  if (value.node_scope !== node.node_class) throw new Error(`manifest node_scope must be ${node.node_class}`);
  if (!isRecord(value.bundle) || value.bundle.root !== BUNDLE_ROOT || Object.keys(value.bundle).length !== 1) {
    throw new Error(`manifest bundle.root must be ${BUNDLE_ROOT}`);
  }
  if (!isRecord(value.agent)) throw new Error("manifest agent must be an object");
  for (const field of Object.keys(value.agent)) {
    if (!AGENT_FIELDS.has(field)) throw new Error(`manifest agent field ${field} is not allowed`);
  }
  if (!Array.isArray(value.tunnels)) throw new Error("manifest tunnels must be an array");
  for (const tunnel of value.tunnels) validateTunnel(tunnel);
}

function validateTunnel(value: unknown): void {
  if (!isRecord(value) || typeof value.contract !== "string" || !CONTRACTS.has(value.contract)) {
    throw new Error("each tunnel requires a supported typed contract");
  }
  if (typeof value.id !== "string" || value.id.length === 0 || value.id.length > 128) {
    throw new Error("each tunnel requires a bounded id");
  }
  if (value.id.startsWith("owned-")) throw new Error("owned-* IDs are reserved for the lane editor");
}

function rejectProtectedValues(value: unknown, path: string): void {
  if (typeof value === "string") {
    if (/-----BEGIN(?: [A-Z]+)? PRIVATE KEY-----/.test(value)) {
      throw new Error(`inline private key is not allowed at ${path}`);
    }
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((item, index) => rejectProtectedValues(item, `${path}[${index}]`));
    return;
  }
  if (!isRecord(value)) return;
  for (const [field, nested] of Object.entries(value)) {
    if (isProtectedField(field)) throw new Error(`inline protected field ${path}.${field} is not supported`);
    rejectProtectedValues(nested, `${path}.${field}`);
  }
}

function isProtectedField(field: string): boolean {
  const normalized = field.toLowerCase().replaceAll("-", "_");
  return PROTECTED_FIELDS.has(normalized) || normalized.endsWith("_token") || normalized.endsWith("_secret");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function utf8Bytes(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}
