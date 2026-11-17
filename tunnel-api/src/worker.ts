import { handleBootstrap, handleBundleActivation, handleBundleRelease } from "./bundles";
import { handleRelayDomesticCapsule } from "./capsules";
import { handleControlKey } from "./control";
import { handleHeartbeats } from "./heartbeats";
import { error, json } from "./http";
import type { Env } from "./models";
import { handleEnroll, handleNodeDocument } from "./nodes";
import { handleRelayDocuments } from "./relay";
import { handleTelegram } from "./telegram";
import { handleRelayTelemetry, handleTelemetry } from "./telemetry";
import { handleWebCluster, handleWebClusters } from "./web";

async function handleApi(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  const url = new URL(request.url);
  if (url.pathname === "/api/health") return json({ ok: true, service: "tunnel-agent" });
  if (url.pathname === "/api/telegram/webhook") return handleTelegram(request, env, ctx);
  if (url.pathname === "/api/v1/web/clusters") return handleWebClusters(request, env);
  if (url.pathname === "/api/v1/bootstrap") return handleBootstrap(request, env);
  if (url.pathname === "/api/v1/enroll") return handleEnroll(request, env);
  if (url.pathname === "/api/v1/admin/bundles/activate") return handleBundleActivation(request, env);
  const document = url.pathname.match(/^\/api\/v1\/nodes\/([0-9a-f-]{36})\/document$/);
  if (document) return handleNodeDocument(request, env, document[1]);
  const bundleRelease = url.pathname.match(/^\/api\/v1\/nodes\/([0-9a-f-]{36})\/bundle-release$/);
  if (bundleRelease) return handleBundleRelease(request, env, bundleRelease[1]);
  const controlKey = url.pathname.match(/^\/api\/v1\/nodes\/([0-9a-f-]{36})\/clusters\/([0-9a-f-]{36})\/control-key$/);
  if (controlKey) return handleControlKey(request, env, controlKey[1], controlKey[2]);
  const domesticCapsule = url.pathname.match(/^\/api\/v1\/nodes\/([0-9a-f-]{36})\/clusters\/([0-9a-f-]{36})\/domestic-nodes\/([0-9a-f-]{36})\/capsule$/);
  if (domesticCapsule) return handleRelayDomesticCapsule(request, env, domesticCapsule[1], domesticCapsule[2], domesticCapsule[3]);
  const relayDocuments = url.pathname.match(/^\/api\/v1\/nodes\/([0-9a-f-]{36})\/relay-documents$/);
  if (relayDocuments) return handleRelayDocuments(request, env, relayDocuments[1]);
  const heartbeats = url.pathname.match(/^\/api\/v1\/nodes\/([0-9a-f-]{36})\/heartbeats$/);
  if (heartbeats) return handleHeartbeats(request, env, heartbeats[1]);
  const telemetry = url.pathname.match(/^\/api\/v1\/nodes\/([0-9a-f-]{36})\/telemetry$/);
  if (telemetry) return handleTelemetry(request, env, telemetry[1]);
  const relayTelemetry = url.pathname.match(/^\/api\/v1\/nodes\/([0-9a-f-]{36})\/relay-telemetry$/);
  if (relayTelemetry) return handleRelayTelemetry(request, env, relayTelemetry[1]);
  const webCluster = url.pathname.match(/^\/api\/v1\/web\/clusters\/([0-9a-f-]{36})$/);
  if (webCluster) return handleWebCluster(request, env, webCluster[1]);
  return error("not found", 404);
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    if (new URL(request.url).pathname.startsWith("/api/")) return handleApi(request, env, ctx);
    return error("not found", 404);
  },
};
