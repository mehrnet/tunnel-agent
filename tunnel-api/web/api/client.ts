import type { ClusterResponse, ClusterSummary } from "../data/types";

const apiRoot = "/api/v1/web";

function requestHeaders(initData: string) {
  return { "x-telegram-init-data": initData };
}

async function readResponse<T>(path: string, initData: string): Promise<T> {
  const response = await fetch(path, {
    credentials: "same-origin",
    headers: requestHeaders(initData),
  });
  if (!response.ok) {
    if (response.status === 401) throw new Error("Telegram session expired or invalid. Close this window and reopen it from TunnelOwnersBot.");
    throw new Error(`Cluster request failed (HTTP ${response.status}). Please retry.`);
  }
  return response.json() as Promise<T>;
}

export function fetchClusters(initData: string) {
  return readResponse<{ clusters: ClusterSummary[]; release_operator: boolean }>(`${apiRoot}/clusters`, initData);
}

export function fetchCluster(id: string, initData: string): Promise<ClusterResponse> {
  return readResponse<ClusterResponse>(`${apiRoot}/clusters/${encodeURIComponent(id)}`, initData);
}

export async function postOwner(path: string, initData: string, body: Record<string, unknown>) {
  const reply = await fetch(`${apiRoot}${path}`, {
    method: "POST", credentials: "same-origin",
    headers: { ...requestHeaders(initData), "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!reply.ok) {
    const details = await reply.json().catch(() => null) as { error?: string } | null;
    throw new Error(details?.error ?? `Request failed (HTTP ${reply.status})`);
  }
  return reply;
}
