import { error, json, parseJson } from "./http";
import type { Env } from "./models";
import { authorizeNode } from "./nodes";
import { foreignCanObserveDomestic, recordNodeHeartbeat } from "./heartbeats-store";
import { nodeById } from "./store";
import { now } from "./validation";

const MAX_OBSERVATIONS = 64;
const NODE_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const MAX_FUTURE_SECONDS = 5 * 60;

interface Observation {
  nodeId: string;
  observedAt: number;
}

export async function handleHeartbeats(request: Request, env: Env, foreignNodeId: string): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  if (!await authorizeNode(request, env, foreignNodeId)) return error("unauthorized", 401);
  const foreign = await nodeById(env, foreignNodeId);
  if (!foreign || foreign.node_class !== "foreign" || foreign.status !== "enrolled") {
    return error("only enrolled foreign relays may report heartbeats", 403);
  }
  const observations = parseObservations(await parseJson(request));
  if (!observations) return error("invalid heartbeat observations", 400);
  if (observations.some((observation) => observation.observedAt > now() + MAX_FUTURE_SECONDS)) {
    return error("heartbeat timestamp is in the future", 400);
  }
  const permitted = await Promise.all(observations.map((observation) => (
    foreignCanObserveDomestic(env, foreignNodeId, observation.nodeId)
  )));
  if (permitted.some((allowed) => !allowed)) return error("heartbeat target is outside this relay's clusters", 403);
  await Promise.all(observations.map((observation) => (
    recordNodeHeartbeat(env, observation.nodeId, foreignNodeId, observation.observedAt)
  )));
  return json({ accepted: observations.length });
}

function parseObservations(body: Record<string, unknown> | null): Observation[] | null {
  const values = body?.observations;
  if (!Array.isArray(values) || values.length < 1 || values.length > MAX_OBSERVATIONS) return null;
  const observations = values.map((value) => {
    if (value === null || typeof value !== "object" || Array.isArray(value)) return null;
    const record = value as Record<string, unknown>;
    if (typeof record.node_id !== "string" || !NODE_ID.test(record.node_id)) return null;
    if (typeof record.observed_at !== "number" || !Number.isSafeInteger(record.observed_at) || record.observed_at <= 0) return null;
    return { nodeId: record.node_id, observedAt: record.observed_at };
  });
  return observations.some((observation) => observation === null) ? null : observations as Observation[];
}
