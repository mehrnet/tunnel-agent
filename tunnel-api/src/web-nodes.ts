import { error, json } from "./http";
import type { Env } from "./models";
import { nodeManifest } from "./nodes";
import { attachNodeToCluster, clusterById, clusterMembershipCount, createNodeForOwner, nodeByName, nodeForOwnerById } from "./store";
import { installCommand } from "./telegram-flow";
import { NODE_NAME } from "./validation";
import { ownerFromTelegram } from "./web-auth";

export async function handleWebNode(request: Request, env: Env, nodeId?: string): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  const owner = await ownerFromTelegram(request, env);
  if (!owner) return error("Telegram authorization required", 401);
  if (nodeId) {
    const node = await nodeForOwnerById(env, owner.id, nodeId);
    if (!node || node.status === "revoked" || await clusterMembershipCount(env, nodeId) < 1) return error("Server unavailable", 404);
    return new Response(JSON.stringify({ command: await installCommand(env, owner, node) }), { headers: { "content-type": "application/json", "cache-control": "no-store" } });
  }
  const body = await request.json().catch(() => null) as Record<string, unknown> | null;
  const name = typeof body?.name === "string" ? body.name.trim().toLowerCase() : "";
  const role = body?.role;
  const address = typeof body?.address === "string" ? body.address.trim() : "";
  if (!NODE_NAME.test(name) || !["domestic", "foreign"].includes(String(role)) || !address || address.length > 253 || /\s/.test(address)) return error("Invalid server name, role or address", 400);
  if (typeof body?.cluster_id !== "string" || !await clusterById(env, owner.id, body.cluster_id)) return error("Cluster not found", 404);
  if (await nodeByName(env, owner.id, name)) return error("Server name already exists", 409);
  const nodeClass = role === "domestic" ? "domestic" : "foreign";
  const node = await createNodeForOwner(env, owner.id, name, nodeClass, address, nodeManifest(nodeClass));
  await attachNodeToCluster(env, body.cluster_id, node.id, nodeClass);
  return json({ node });
}
