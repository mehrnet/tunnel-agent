import { error, json, parseJson } from "./http";
import type { Env } from "./models";
import { cancelProbe, startProbe } from "./probe-store";
import { clusterById } from "./store";
import { ownerFromTelegram } from "./web-auth";

export async function handleWebProbes(request: Request, env: Env, cluster: string) {
  if (request.method !== "POST") return error("method not allowed", 405);
  const owner = await ownerFromTelegram(request, env);
  if (!owner) return error("Telegram authorization required", 401);
  if (!await clusterById(env, owner.id, cluster)) return error("Cluster not found", 404);
  const body = await parseJson(request);
  if (!body) return error("Invalid request body", 400);
  try {
    if (body.action === "start") return json(await startProbe(env, cluster, body), 201);
    if (body.action === "cancel") return json(await cancelProbe(env, cluster, body.id));
    return error("Unknown probe action", 400);
  } catch (reason) {
    const message = reason instanceof Error ? reason.message : "";
    return error(!message || /D1_|constraint failed/i.test(message) ? "Probe configuration changed; reload and retry" : message, 409);
  }
}
