import { clusterControlPrivateKey } from "./cluster-keys";
import { error, json } from "./http";
import { authorizeNode } from "./nodes";
import { foreignClusterMember, nodeById } from "./store";
import type { Env } from "./models";

export async function handleControlKey(
  request: Request,
  env: Env,
  nodeId: string,
  clusterId: string,
): Promise<Response> {
  if (request.method !== "GET") return error("method not allowed", 405);
  if (!await authorizeNode(request, env, nodeId)) return error("unauthorized", 401);
  const node = await nodeById(env, nodeId);
  if (!node || node.status !== "enrolled" || !await foreignClusterMember(env, nodeId, clusterId)) {
    return error("foreign cluster membership is required", 403);
  }
  try {
    return json({ private_key: await clusterControlPrivateKey(env, clusterId) });
  } catch (reason) {
    return error(reason instanceof Error ? reason.message : "control key is unavailable", 503);
  }
}
