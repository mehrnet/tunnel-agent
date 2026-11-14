import { clusterControlPublicKey } from "./cluster-keys";
import { base62Capsule, base64Standard, base64Url } from "./encoding";
import type { BundleRow, Env, NodeRow } from "./models";
import { activeBundle, foreignRelayAddresses, nodeClusters, nodeDocument } from "./store";
import { filesOrigin, now } from "./validation";

interface CapsuleBundle {
  v: string;
  h: string;
  u: string;
}

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
    activeBundle(env, "amd64"),
    activeBundle(env, "arm64"),
  ]);
  if (!amd64 || !arm64 || relays.length === 0) {
    throw new Error("domestic offline bootstrap requires active bundles and at least one foreign relay address");
  }
  requireDomesticBundles(env, amd64, arm64);
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
  return base62Capsule(JSON.stringify({ p: base64Url(new TextEncoder().encode(payload)), s: await sign(env, payload) }));
}

function requireDomesticBundles(env: Env, amd64: BundleRow, arm64: BundleRow): void {
  const expected = env.DOMESTIC_CAPSULE_BUNDLE_VERSION?.trim();
  if (!expected || amd64.version !== expected || arm64.version !== expected ||
    !amd64.domestic_url || !arm64.domestic_url) {
    throw new Error("domestic bootstrap is awaiting matching active Iranian mirror bundles");
  }
}

function capsuleTtl(env: Env): number {
  const value = Number.parseInt(env.DOMESTIC_CAPSULE_TTL_SECONDS ?? "86400", 10);
  return Number.isSafeInteger(value) && value >= 300 && value <= 604_800 ? value : 86_400;
}

function bundleUrls(env: Env, bundle: BundleRow): string {
  const publicUrl = new URL(bundle.public_url, filesOrigin(env)).toString();
  return [bundle.domestic_url, publicUrl].filter((url): url is string => Boolean(url)).join(",");
}

function relayEndpoint(relay: { address: string }): Relay {
  if (!/^[A-Za-z0-9.-]{1,253}$/.test(relay.address)) {
    throw new Error(`foreign relay address is invalid: ${relay.address}`);
  }
  return { h: relay.address, p: 2222 };
}

async function sign(env: Env, payload: string): Promise<string> {
  const privateKey = base64Standard(env.CAPSULE_SIGNING_PRIVATE_KEY ?? "");
  if (!privateKey) throw new Error("capsule signing is not configured");
  const key = await crypto.subtle.importKey("pkcs8", privateKey, { name: "Ed25519" }, false, ["sign"]);
  const signature = await crypto.subtle.sign("Ed25519", key, new TextEncoder().encode(payload));
  return base64Url(signature);
}
