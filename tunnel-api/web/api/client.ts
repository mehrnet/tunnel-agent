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
  if (!response.ok) throw new Error(`خطای دریافت داده (${response.status})`);
  return response.json() as Promise<T>;
}

export async function fetchClusters(initData: string): Promise<ClusterSummary[]> {
  const payload = await readResponse<{ clusters: ClusterSummary[] }>(`${apiRoot}/clusters`, initData);
  return payload.clusters;
}

export function fetchCluster(id: string, initData: string): Promise<ClusterResponse> {
  return readResponse<ClusterResponse>(`${apiRoot}/clusters/${encodeURIComponent(id)}`, initData);
}
