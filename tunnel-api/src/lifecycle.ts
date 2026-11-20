import { error, json } from "./http";
import { transitionNode } from "./lifecycle-store";
import type { Env } from "./models";
import { nodeForOwnerById } from "./store";
import { ownerFromTelegram } from "./web-auth";

export async function handleNodeLifecycle(request: Request, env: Env, nodeId: string): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  const owner = await ownerFromTelegram(request, env);
  if (!owner) return error("Telegram authorization required", 401);
  const node = await nodeForOwnerById(env, owner.id, nodeId);
  if (!node) return error("server not found", 404);
  const body = await request.json().catch(() => null) as { action?: unknown; confirm_node_id?: unknown } | null;
  const action = body?.action;
  if (action !== "reinstall" && action !== "rotate" && action !== "retire") return error("invalid lifecycle action", 400);
  if (body?.confirm_node_id !== nodeId) return error("explicit server confirmation required", 400);
  try {
    await transitionNode(env, owner, node, action);
    return json({ node_id: nodeId, state: action === "retire" ? "revoked" : "pending", requires_bootstrap: action !== "retire" });
  } catch (reason) { return error(reason instanceof Error ? reason.message : "transition failed", 409); }
}
