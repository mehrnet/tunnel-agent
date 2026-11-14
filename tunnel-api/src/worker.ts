import { handleBootstrap, handleBundleActivation, handleBundleRelease } from "./bundles";
import { error, json } from "./http";
import type { Env } from "./models";
import { handleEnroll, handleNodeDocument } from "./nodes";
import { handleRelayDocuments } from "./relay";
import { handleTelegram } from "./telegram";

async function handleApi(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  const url = new URL(request.url);
  if (url.pathname === "/api/health") return json({ ok: true, service: "tunnel-agent" });
  if (url.pathname === "/api/telegram/webhook") return handleTelegram(request, env, ctx);
  if (url.pathname === "/api/v1/bootstrap") return handleBootstrap(request, env);
  if (url.pathname === "/api/v1/enroll") return handleEnroll(request, env);
  if (url.pathname === "/api/v1/admin/bundles/activate") return handleBundleActivation(request, env);
  const document = url.pathname.match(/^\/api\/v1\/nodes\/([0-9a-f-]{36})\/document$/);
  if (document) return handleNodeDocument(request, env, document[1]);
  const bundleRelease = url.pathname.match(/^\/api\/v1\/nodes\/([0-9a-f-]{36})\/bundle-release$/);
  if (bundleRelease) return handleBundleRelease(request, env, bundleRelease[1]);
  const relayDocuments = url.pathname.match(/^\/api\/v1\/nodes\/([0-9a-f-]{36})\/relay-documents$/);
  if (relayDocuments) return handleRelayDocuments(request, env, relayDocuments[1]);
  return error("not found", 404);
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    if (new URL(request.url).pathname.startsWith("/api/")) return handleApi(request, env, ctx);
    return error("not found", 404);
  },
};
