import { error, json, message, parseJson } from "./http";
import type { Architecture, BundleActivation, BundleActivationEntry, BundleRow, Env, NodeClass } from "./models";
import { trackedBundle } from "./release-tracks";
import { nodeById, requireGrant } from "./store";
import { authorizeNode } from "./nodes";
import { base64Url } from "./encoding";
import { signApiPayload } from "./signing";
import { BUNDLE_VERSION, equalSecret, filesOrigin, HEX_64, now, validArchitecture } from "./validation";

const MAX_DOMESTIC_MIRRORS = 4;
const RELEASE_VERSION = /^v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.\d$/;

function bundleTarget(architecture: Architecture): string {
  return architecture === "amd64" ? "x86_64-unknown-linux-musl" : "aarch64-unknown-linux-musl";
}

function bundleArchivePath(version: string, architecture: Architecture): string {
  return `/bundles/${version}/tunnel-agent-${version}-${bundleTarget(architecture)}.tar.gz`;
}

function bundlePublicPath(version: string, architecture: Architecture): string {
  return `${bundleArchivePath(version, architecture)}.parts`;
}

function validBundleSourceUrl(value: string): boolean {
  try {
    const url = new URL(value);
    const trustedDomesticHttp = url.protocol === "http:" && url.hostname === "dl.mehr-mir.ir" && !url.port;
    return (url.protocol === "https:" || trustedDomesticHttp) && !url.username && !url.password && !url.search && !url.hash &&
      !/[\s,]/.test(value);
  } catch {
    return false;
  }
}

function validDomesticUrls(value: unknown, legacyValue: unknown): string[] | null {
  const rawUrls = value === undefined
    ? legacyValue === null || legacyValue === undefined ? [] : [legacyValue]
    : value;
  if (!Array.isArray(rawUrls) || rawUrls.length > MAX_DOMESTIC_MIRRORS) return null;
  const urls = rawUrls.filter((url): url is string => typeof url === "string");
  return urls.length === rawUrls.length && new Set(urls).size === urls.length &&
    urls.every(validBundleSourceUrl) ? urls : null;
}

export function registeredDomesticUrls(bundle: BundleRow): string[] | null {
  if (bundle.domestic_urls === null) return validDomesticUrls(undefined, bundle.domestic_url);
  try {
    return validDomesticUrls(JSON.parse(bundle.domestic_urls), undefined);
  } catch {
    return null;
  }
}

function parseBundleEntry(
  rawEntry: unknown,
  expectedArchitectures: Set<Architecture>,
  version: string,
): BundleActivationEntry | null {
  if (!rawEntry || typeof rawEntry !== "object" || Array.isArray(rawEntry)) return null;
  const entry = rawEntry as Record<string, unknown>;
  const architecture = typeof entry.architecture === "string" ? entry.architecture : undefined;
  const sha256 = typeof entry.sha256 === "string" ? entry.sha256 : "";
  const inventorySha256 = typeof entry.inventory_sha256 === "string" ? entry.inventory_sha256 : "";
  const publicUrl = typeof entry.public_url === "string" ? entry.public_url : "";
  const domesticUrls = validDomesticUrls(entry.domestic_urls, entry.domestic_url);
  if (!validArchitecture(architecture) || !expectedArchitectures.delete(architecture) ||
    !HEX_64.test(sha256) || !HEX_64.test(inventorySha256) || domesticUrls === null) return null;

  const expectedManifestPath = bundlePublicPath(version, architecture);
  if (publicUrl !== expectedManifestPath) return null;
  return { architecture, sha256, inventory_sha256: inventorySha256, public_url: publicUrl, domestic_urls: domesticUrls };
}

function parseBundleActivation(body: Record<string, unknown> | null): BundleActivation | null {
  const version = typeof body?.version === "string" ? body.version : "";
  const track = body?.track;
  if ((track !== "stable" && track !== "testing") || !RELEASE_VERSION.test(version) ||
    !Array.isArray(body?.bundles) || body.bundles.length !== 2) return null;

  const expectedArchitectures = new Set<Architecture>(["amd64", "arm64"]);
  const bundles: BundleActivationEntry[] = [];
  for (const rawEntry of body.bundles) {
    const entry = parseBundleEntry(rawEntry, expectedArchitectures, version);
    if (!entry) return null;
    bundles.push(entry);
  }
  return expectedArchitectures.size === 0 ? { version, bundles, track } : null;
}

function bootstrapEnv(bundle: BundleRow, nodeClass: NodeClass, origin: string): string {
  if (!HEX_64.test(bundle.sha256)) throw new Error("bundle registry has invalid SHA-256");
  const publicUrl = new URL(bundle.public_url, origin).toString();
  const domesticUrls = registeredDomesticUrls(bundle);
  if (domesticUrls === null) throw new Error("bundle registry has invalid domestic mirror URLs");
  const sources = nodeClass === "domestic" ? [...domesticUrls, publicUrl] : [publicUrl];
  if (sources.some((source) => !validBundleSourceUrl(source))) {
    throw new Error("bundle registry has invalid source URL");
  }
  if (!BUNDLE_VERSION.test(bundle.version)) throw new Error("bundle registry has invalid version");
  return [
    `TUNNEL_AGENT_BUNDLE_VERSION=${bundle.version}`,
    `TUNNEL_AGENT_BUNDLE_SHA256=${bundle.sha256}`,
    `TUNNEL_AGENT_BUNDLE_URLS=${sources.join(",")}`,
  ].join("\n") + "\n";
}

export async function handleBootstrap(request: Request, env: Env): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  const body = await parseJson(request);
  const grant = await requireGrant(env, body?.grant);
  const requestedArchitecture = typeof body?.architecture === "string" ? body.architecture : undefined;
  if (!grant || !validArchitecture(requestedArchitecture)) {
    return error("invalid enrollment grant or architecture", 401);
  }
  const bundle = await trackedBundle(env, grant.id, requestedArchitecture);
  if (!bundle) return error(`no active ${requestedArchitecture} tunnel-agent bundle`, 503);
  try {
    return message(bootstrapEnv(bundle, grant.node_class, filesOrigin(env)));
  } catch (reason) {
    return error(reason instanceof Error ? reason.message : "invalid bundle registry", 503);
  }
}

export async function handleBundleActivation(request: Request, env: Env): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  const authorization = request.headers.get("authorization");
  const token = authorization?.startsWith("Bearer ") ? authorization.slice("Bearer ".length) : null;
  if (!equalSecret(token, env.BUNDLE_PUBLISH_TOKEN)) return error("unauthorized", 401);

  const activation = parseBundleActivation(await parseJson(request));
  if (!activation) return error("invalid bundle activation payload", 400);
  if (activation.track === "stable" && activation.bundles.some((bundle) => !bundle.domestic_urls.length)) {
    return error("stable promotion requires domestic mirrors for both architectures", 400);
  }
  if (activation.track === "stable") {
    const tested = await env.DB.prepare(
      "SELECT COUNT(*) AS count FROM bundle_releases WHERE version=? AND (testing=1 OR active=1)",
    ).bind(activation.version).first<number>("count");
    if (tested !== 2) return error("Publish this version to testing before promoting it to stable", 409);
  }

  const timestamp = now();
  const statements: D1PreparedStatement[] = [];
  const column = activation.track === "stable" ? "active" : "testing";
  for (const bundle of activation.bundles) {
    statements.push(
      env.DB.prepare(`UPDATE bundle_releases SET ${column}=0 WHERE architecture=? AND ${column}=1`)
        .bind(bundle.architecture),
      env.DB.prepare(
        `INSERT INTO bundle_releases
          (version, architecture, sha256, inventory_sha256, public_url, domestic_url, domestic_urls, ${column}, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?)
         ON CONFLICT(version, architecture) DO UPDATE SET
           sha256 = CASE WHEN sha256=excluded.sha256 AND inventory_sha256 IS excluded.inventory_sha256
                         THEN sha256 ELSE NULL END,
           domestic_url = CASE WHEN active=1 THEN domestic_url ELSE COALESCE(excluded.domestic_url, domestic_url) END,
           domestic_urls = CASE WHEN active=1 OR excluded.domestic_url IS NULL THEN domestic_urls ELSE excluded.domestic_urls END,
           ${column} = 1`,
      ).bind(
        activation.version,
        bundle.architecture,
        bundle.sha256,
        bundle.inventory_sha256,
        bundle.public_url,
        bundle.domestic_urls[0] ?? null,
        JSON.stringify(bundle.domestic_urls),
        timestamp,
      ),
    );
  }
  if (activation.track === "testing") statements.push(env.DB.prepare("DELETE FROM seed_canary_releases"));
  statements.push(env.DB.prepare(
    `INSERT INTO audit_events (id, owner_id, node_id, cluster_id, action, detail_json, created_at)
     VALUES (?, NULL, NULL, NULL, ?, ?, ?)`,
  ).bind(
    crypto.randomUUID(),
    "bundle.activated",
    JSON.stringify({ version: activation.version, track: activation.track }),
    timestamp,
  ));
  try { await env.DB.batch(statements); }
  catch { return error("Release metadata conflicts with an immutable version", 409); }
  return json({ ok: true, version: activation.version, track: activation.track });
}

export async function handleBundleRelease(request: Request, env: Env, nodeId: string): Promise<Response> {
  if (request.method !== "GET") return error("method not allowed", 405);
  if (!await authorizeNode(request, env, nodeId)) return error("unauthorized", 401);
  const node = await nodeById(env, nodeId);
  if (!node || node.node_class !== "foreign" || node.status !== "enrolled") {
    return error("only enrolled foreign nodes may fetch a release document", 403);
  }
  const architecture = new URL(request.url).searchParams.get("architecture") ?? undefined;
  if (!validArchitecture(architecture)) return error("invalid architecture", 400);

  const bundle = await trackedBundle(env, nodeId, architecture);
  if (!bundle || !BUNDLE_VERSION.test(bundle.version) || !HEX_64.test(bundle.sha256) ||
    !bundle.inventory_sha256 || !HEX_64.test(bundle.inventory_sha256)) {
    return error(`no seedable ${architecture} tunnel-agent bundle`, 503);
  }
  const payload = JSON.stringify({
    v: 1,
    a: bundle.architecture,
    b: bundle.version,
    h: bundle.sha256,
    i: bundle.inventory_sha256,
  });
  try {
    return json({
      envelope: JSON.stringify({
        p: base64Url(new TextEncoder().encode(payload)),
        s: await signApiPayload(env, payload),
      }),
    });
  } catch (reason) {
    return error(reason instanceof Error ? reason.message : "release signing failed", 503);
  }
}
