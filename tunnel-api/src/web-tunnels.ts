import { error, json, parseJson } from "./http";
import type { Env } from "./models";
import { clusterById } from "./store";
import { destination, parseIntent, type Intent } from "./tunnel-spec";
import { getIntent, reconcileTunnel } from "./tunnel-store";
import { randomToken } from "./validation";
import { ownerFromTelegram } from "./web-auth";

export async function handleWebTunnel(request: Request, env: Env, clusterId: string, id?: string): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  const owner = await ownerFromTelegram(request, env);
  if (!owner) return error("Telegram authorization required", 401);
  if (!await clusterById(env, owner.id, clusterId)) return error("Cluster not found", 404);
  const body = await parseJson(request);
  if (!body) return error("Invalid request body", 400);
  try {
    if (!id) {
      const intent = parseIntent(body, clusterId);
      await reconcileTunnel(env, intent, "create");
      return json({ id: intent.id, enabled: false, revision: 1 }, 201);
    }
    const intent = await getIntent(env, clusterId, id);
    if (!intent) return error("Managed lane not found", 404);
    if (body.revision !== intent.revision) return error("Lane changed; reload before editing", 409);
    return await changeTunnel(env, intent, body);
  } catch (reason) {
    if (!(reason instanceof Error)) return error("Could not save lane", 409);
    if (/D1_|constraint failed/i.test(reason.message)) return error("Lane or port reservation changed; reload and retry", 409);
    return error(reason.message, 409);
  }
}

async function changeTunnel(env: Env, intent: Intent, body: Record<string, unknown>) {
  switch (body.action) {
    case "enable":
      if (env.MANAGED_TUNNEL_ACTIVATION !== "1") return error("Activation requires the updated agent bundle; this lane remains saved and disabled", 409);
      intent.enabled = 1;
      break;
    case "disable": intent.enabled = 0; break;
    case "restart":
      if (!intent.enabled) return error("Enable the lane before restarting it", 409);
      intent.path_key = randomToken();
      break;
    case "destination":
      if (intent.listener_type !== "tcp") return error("SOCKS5 has no fixed destination", 400);
      Object.assign(intent, destination(body));
      break;
    case "remove":
      if (intent.enabled) return error("Disable the lane before removing it", 409);
      await reconcileTunnel(env, intent, "remove");
      return json({ removed: true });
    default: return error("Unknown lane action", 400);
  }
  await reconcileTunnel(env, intent, "update");
  return json({ enabled: intent.enabled === 1, revision: intent.revision + 1 });
}
