import { base64Url } from "./encoding";
import { error, json } from "./http";
import type { Env, RelayDocumentRow } from "./models";
import { authorizeNode } from "./nodes";
import { signApiPayload } from "./signing";
import { nodeById, relayDocumentsForForeignNode } from "./store";
import { sha256 } from "./validation";

interface RelayEnvelope {
  cluster_id: string;
  node_id: string;
  revision: number;
  envelope: string;
}

export async function handleRelayDocuments(request: Request, env: Env, nodeId: string): Promise<Response> {
  if (request.method !== "GET") return error("method not allowed", 405);
  if (!await authorizeNode(request, env, nodeId)) return error("unauthorized", 401);
  const node = await nodeById(env, nodeId);
  if (!node || node.node_class !== "foreign" || node.status !== "enrolled") {
    return error("only enrolled foreign relays may fetch relay documents", 403);
  }

  try {
    const documents = await relayDocumentsForForeignNode(env, nodeId);
    const etag = await documentTag(env, documents);
    if (request.headers.get("if-none-match") === etag) {
      return new Response(null, { status: 304, headers: { etag, "cache-control": "no-store" } });
    }
    const envelopes = await Promise.all(documents.map((document) => issueEnvelope(env, document)));
    const response = json({ documents: envelopes });
    response.headers.set("etag", etag);
    return response;
  } catch (reason) {
    const detail = reason instanceof Error ? reason.message : "relay document issuance failed";
    return error(detail, 503);
  }
}

async function documentTag(env: Env, documents: RelayDocumentRow[]): Promise<string> {
  const signer = await sha256(env.CAPSULE_SIGNING_PRIVATE_KEY ?? "");
  const revisions = documents.map((row) => [row.cluster_id, row.node_id, row.revision, row.updated_at]);
  return `"${await sha256(JSON.stringify([1, signer, revisions]))}"`;
}

async function issueEnvelope(env: Env, document: RelayDocumentRow): Promise<RelayEnvelope> {
  const payload = JSON.stringify({
    v: 1,
    c: document.cluster_id,
    n: document.node_id,
    i: document.revision,
    m: base64Url(new TextEncoder().encode(document.manifest_json)),
  });
  const envelope = JSON.stringify({
    p: base64Url(new TextEncoder().encode(payload)),
    s: await signApiPayload(env, payload),
  });
  return {
    cluster_id: document.cluster_id,
    node_id: document.node_id,
    revision: document.revision,
    envelope,
  };
}
