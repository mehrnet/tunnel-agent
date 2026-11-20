import { error, json, parseJson } from "./http";
import type { Architecture, BundleRow, Env } from "./models";
import { activeBundle, clusterById } from "./store";
import { ownerFromTelegram } from "./web-auth";

export async function handleReleaseTrack(request: Request, env: Env, clusterId: string) {
  if (request.method !== "POST") return error("method not allowed", 405);
  const owner = await ownerFromTelegram(request, env);
  if (!owner) return error("Telegram authorization required", 401);
  if (!await clusterById(env, owner.id, clusterId)) return error("Cluster not found", 404);
  const track = (await parseJson(request))?.track;
  if (track === "stable") {
    await env.DB.prepare("DELETE FROM owner_testing_clusters WHERE owner_id=? AND cluster_id=?").bind(owner.id, clusterId).run();
  } else if (track === "testing") {
    await env.DB.prepare(
      `INSERT INTO owner_testing_clusters(owner_id,cluster_id)
       SELECT owner_id,id FROM clusters WHERE owner_id=? AND id=?
       ON CONFLICT(owner_id) DO UPDATE SET cluster_id=excluded.cluster_id`,
    ).bind(owner.id, clusterId).run();
  } else {
    return error("Track must be stable or testing", 400);
  }
  return json({ track });
}

async function isTestingNode(env: Env, nodeId: string): Promise<boolean> {
  return Boolean(await env.DB.prepare(
    `SELECT 1 FROM owner_testing_clusters t
     JOIN cluster_nodes m ON m.cluster_id=t.cluster_id
     JOIN clusters c ON c.id=t.cluster_id AND c.owner_id=t.owner_id
     WHERE m.node_id=? AND NOT EXISTS (
       SELECT 1 FROM cluster_nodes a JOIN cluster_nodes b ON b.node_id=a.node_id
       WHERE a.cluster_id=t.cluster_id AND b.cluster_id!=t.cluster_id
     )`,
  ).bind(nodeId).first());
}

export async function trackedBundle(env: Env, nodeId: string, architecture: Architecture): Promise<BundleRow | null> {
  if (await isTestingNode(env, nodeId)) {
    const candidate = await env.DB.prepare(
      `SELECT version,architecture,sha256,inventory_sha256,public_url,domestic_url,domestic_urls
       FROM bundle_releases WHERE architecture=? AND testing=1`,
    ).bind(architecture).first<BundleRow>();
    if (candidate) return candidate;
  }
  return activeBundle(env, architecture);
}
