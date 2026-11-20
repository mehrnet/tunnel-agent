import { clusterControlPublicKey } from "./cluster-keys";
import { registeredDomesticUrls } from "./bundles";
import { base62Capsule, base64Url } from "./encoding";
import { error, json } from "./http";
import type { BundleRow, Env, NodeRow } from "./models";
import { authorizeNode } from "./nodes";
import { signApiPayload } from "./signing";
import { trackedBundle } from "./release-tracks";
import { audit, foreignClusterMember, foreignRelayAddresses, nodeById, nodeClusters, nodeDocument } from "./store";
import { filesOrigin, now } from "./validation";

interface Relay {
  h: string;
  p: number;
}

export async function issueDomesticCapsule(env: Env, node: NodeRow): Promise<string> {
  const [clusters, document] = await Promise.all([nodeClusters(env, node.id), nodeDocument(env, node.id)]);
  if (clusters.length !== 1 || !document) {
    throw new Error("domestic offline bootstrap requires exactly one cluster membership and a node document");
  }
  const [controlKey, relays, amd64, arm64] = await Promise.all([
    clusterControlPublicKey(env, clusters[0].id),
    foreignRelayAddresses(env, clusters[0].id),
    trackedBundle(env, node.id, "amd64"),
    trackedBundle(env, node.id, "arm64"),
  ]);
  if (!amd64 || !arm64 || relays.length === 0) {
    throw new Error("domestic offline bootstrap requires active bundles and at least one foreign relay address");
  }
  requireDomesticBundles(amd64, arm64);
  const payload = JSON.stringify({
    v: 1,
    e: now() + capsuleTtl(env),
    n: node.id,
    c: clusters[0].id,
    i: document.revision,
    k: controlKey,
    m: base64Url(new TextEncoder().encode(document.manifest_json)),
    xv: amd64.version,
    xh: amd64.sha256,
    xu: bundleUrls(env, amd64),
    av: arm64.version,
    ah: arm64.sha256,
    au: bundleUrls(env, arm64),
    l: relays.map(relayEndpoint),
  });
  return base62Capsule(JSON.stringify({ p: base64Url(new TextEncoder().encode(payload)), s: await signApiPayload(env, payload) }));
}

export async function handleRelayDomesticCapsule(
  request: Request,
  env: Env,
  relayNodeId: string,
  clusterId: string,
  domesticNodeId: string,
): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  if (!await authorizeNode(request, env, relayNodeId)) return error("unauthorized", 401);
  if (!await foreignClusterMember(env, relayNodeId, clusterId)) return error("forbidden", 403);

  const domestic = await nodeById(env, domesticNodeId);
  if (!domestic || domestic.node_class !== "domestic" || domestic.status === "revoked") {
    return error("domestic node is unavailable", 404);
  }
  const clusters = await nodeClusters(env, domestic.id);
  if (clusters.length !== 1 || clusters[0].id !== clusterId) return error("forbidden", 403);

  try {
    const capsule = await issueDomesticCapsule(env, domestic);
    await audit(env, "domestic.capsule_issued_by_relay", { domestic_node_id: domestic.id }, {
      nodeId: relayNodeId,
      clusterId,
    });
    return json({ capsule });
  } catch (reason) {
    return error(reason instanceof Error ? reason.message : "cannot issue domestic capsule", 503);
  }
}

function requireDomesticBundles(amd64: BundleRow, arm64: BundleRow): void {
  if (amd64.version !== arm64.version ||
    !registeredDomesticUrls(amd64)?.length || !registeredDomesticUrls(arm64)?.length) {
    throw new Error("domestic bootstrap is awaiting matching active Iranian mirror bundles");
  }
}

function capsuleTtl(env: Env): number {
  const value = Number.parseInt(env.DOMESTIC_CAPSULE_TTL_SECONDS ?? "86400", 10);
  return Number.isSafeInteger(value) && value >= 300 && value <= 604_800 ? value : 86_400;
}

function bundleUrls(env: Env, bundle: BundleRow): string {
  const publicUrl = new URL(bundle.public_url, filesOrigin(env)).toString();
  const domesticUrls = registeredDomesticUrls(bundle);
  if (!domesticUrls?.length) throw new Error("domestic bundle mirror is invalid");
  return [...domesticUrls, publicUrl].join(",");
}

function relayEndpoint(relay: { address: string }): Relay {
  if (!/^[A-Za-z0-9.-]{1,253}$/.test(relay.address)) {
    throw new Error(`foreign relay address is invalid: ${relay.address}`);
  }
  return { h: relay.address, p: 2222 };
}
