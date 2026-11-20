import { error, json, parseJson } from "./http";
import { saveIngress } from "./ingress-store";
import type { Env } from "./models";
import { clusterById } from "./store";
import { ownerFromTelegram } from "./web-auth";

export async function handleWebIngress(request: Request, env: Env, clusterId: string) {
  if (request.method !== "POST") return error("method not allowed", 405);
  const owner = await ownerFromTelegram(request, env);
  if (!owner) return error("Telegram authorization required", 401);
  if (!await clusterById(env, owner.id, clusterId)) return error("Cluster not found", 404);
  if (env.MANAGED_TUNNEL_ACTIVATION !== "1") return error("Managed tunnel activation is disabled", 409);
  const body = await parseJson(request);
  if (!body) return error("Invalid request body", 400);
  try { return json(await saveIngress(env, clusterId, body)); }
  catch (reason) { return error(reason instanceof Error ? reason.message : "Public ingress update failed", 409); }
}
