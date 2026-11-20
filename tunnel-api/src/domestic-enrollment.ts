import { base64Standard } from "./encoding";
import { foreignCanObserveDomestic } from "./heartbeats-store";
import { error, json, parseJson } from "./http";
import type { Env, GrantRow } from "./models";
import { authorizeNode } from "./nodes";
import { nodeById, nodeForOwnerById, requireGrant } from "./store";
import { now, randomToken, sha256 } from "./validation";
import { ownerFromTelegram } from "./web-auth";

const publicKeyPattern = /^[A-Za-z0-9+/]{59}=$/;
const signaturePattern = /^[A-Za-z0-9+/]{86}==$/;
const domain = "tunnel-agent-domestic-enrollment-v1";

export async function handleDomesticGrant(request: Request, env: Env, nodeId: string) {
  if (request.method !== "POST") return error("method not allowed", 405);
  const owner = await ownerFromTelegram(request, env);
  if (!owner) return error("Telegram authorization required", 401);
  const node = await nodeForOwnerById(env, owner.id, nodeId);
  if (!node || node.node_class !== "domestic") return error("Domestic server not found", 404);
  if (node.status !== "pending") return error("Server is not pending identity confirmation", 409);
  const key = (await parseJson(request))?.public_key;
  if (typeof key !== "string" || !decodePublicKey(key)) return error("A canonical Ed25519 SPKI public key is required", 400);
  const id = crypto.randomUUID();
  const token = randomToken();
  const time = now();
  const [created] = await env.DB.batch([
    env.DB.prepare(
      `INSERT INTO enrollment_grants(id,node_id,secret_hash,expires_at,created_at)
       SELECT ?,id,?,?,? FROM nodes WHERE id=? AND owner_id=? AND status='pending' AND node_class='domestic'`,
    ).bind(id, await sha256(token), time + 900, time, node.id, owner.id),
    env.DB.prepare("INSERT INTO domestic_enrollment_keys(grant_id,public_key) SELECT ?,? WHERE changes()=1").bind(id,key),
  ]);
  if (created.meta.changes !== 1) return error("Enrollment state changed", 409);
  return json({ grant: `${id}.${token}` });
}

export async function handleDomesticProof(request: Request, env: Env, relayId: string, nodeId: string) {
  if (request.method !== "POST") return error("method not allowed", 405);
  if (!await authorizeNode(request, env, relayId)) return error("unauthorized", 401);
  const relay = await nodeById(env, relayId);
  if (relay?.node_class !== "foreign" || !await foreignCanObserveDomestic(env, relayId, nodeId)) {
    return error("Relay is not authorized for this domestic node", 403);
  }
  const body = await parseJson(request);
  const grant = await requireGrant(env, body?.grant);
  const key = body?.public_key;
  const signature = body?.signature;
  if (!grant || grant.id !== nodeId || grant.status !== "pending" || grant.node_class !== "domestic") {
    return error("Invalid or expired domestic enrollment grant", 401);
  }
  if (typeof key !== "string" || typeof signature !== "string" ||
    !await verifyProof(nodeId, String(body?.grant), key, signature)) {
    return error("Invalid domestic identity signature", 401);
  }
  try {
    if (!await confirmIdentity(env, grant, relayId, key)) return error("Enrollment changed or expired", 409);
  } catch {
    return error("Cannot confirm identity; generate a new key and grant if this key was already used", 409);
  }
  return json({ node_id: nodeId, state: "enrolled" });
}

function decodePublicKey(key: string) {
  if (!publicKeyPattern.test(key)) return null;
  const bytes = base64Standard(key);
  // Canonical DER prevents alternate serializations from bypassing key reuse protection.
  const prefix = [48, 42, 48, 5, 6, 3, 43, 101, 112, 3, 33, 0];
  if (bytes?.length !== 44 || !prefix.every((byte, i) => bytes[i] === byte) ||
    btoa(String.fromCharCode(...bytes)) !== key) return null;
  return bytes.slice(12);
}

async function verifyProof(nodeId: string, grant: string, key: string, proof: string) {
  const bytes = decodePublicKey(key);
  const signature = signaturePattern.test(proof) ? base64Standard(proof) : null;
  if (!bytes || !signature) return false;
  try {
    const publicKey = await crypto.subtle.importKey("raw", bytes, { name: "Ed25519" }, false, ["verify"]);
    return await crypto.subtle.verify("Ed25519", publicKey, signature, new TextEncoder().encode(
      [domain, nodeId, grant, key].join("\n"),
    ));
  } catch { return false; }
}

async function confirmIdentity(env: Env, grant: GrantRow, relayId: string, key: string): Promise<boolean> {
  const clock = now();
  const [claimed] = await env.DB.batch([
    env.DB.prepare(
      `UPDATE enrollment_grants SET consumed_at=?
       WHERE id=? AND node_id=? AND consumed_at IS NULL AND expires_at>?
       AND EXISTS (SELECT 1 FROM domestic_enrollment_keys WHERE grant_id=enrollment_grants.id AND public_key=?)
       AND EXISTS (SELECT 1 FROM nodes WHERE id=? AND status='pending' AND node_class='domestic')
       AND EXISTS (
         SELECT 1 FROM cluster_nodes a JOIN cluster_nodes b ON b.cluster_id=a.cluster_id
         JOIN nodes relay ON relay.id=a.node_id
         WHERE a.node_id=? AND a.role='foreign' AND relay.status='enrolled' AND b.node_id=? AND b.role='domestic'
       )`,
    ).bind(clock, grant.grant_id, grant.id, clock, key, grant.id, relayId, grant.id),
    env.DB.prepare(
      "INSERT INTO domestic_identities(public_key,node_id,relay_node_id,confirmed_at) SELECT ?,?,?,? WHERE changes()=1",
    ).bind(key, grant.id, relayId, clock),
    env.DB.prepare(
      "UPDATE nodes SET public_key=?,enrolled_at=?,status='enrolled' WHERE id=? AND changes()=1",
    ).bind(key, clock, grant.id),
    env.DB.prepare(
      "UPDATE enrollment_grants SET consumed_at=? WHERE node_id=? AND consumed_at IS NULL AND changes()=1",
    ).bind(clock, grant.id),
  ]);
  return claimed.meta.changes === 1;
}
