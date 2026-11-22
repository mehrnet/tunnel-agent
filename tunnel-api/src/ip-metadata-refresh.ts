import { parseIpMetadata, publicIp, type IpMetadata } from "./ip-metadata";
import type { Env } from "./models";
import { now } from "./validation";

const endpoints = ["https://bgp-lite.mehrnet.com/v1/ip", "https://bgp-api.mehrnet.com/v1/ip"];
const week = 7 * 24 * 3600;
const retry = 30 * 60;
const limit = 18;
const concurrency = 6;

async function lookup(ip: string, checkedAt: number): Promise<IpMetadata | null> {
  for (const endpoint of endpoints) {
    try {
      const url = new URL(endpoint);
      url.searchParams.set("query", ip);
      const response = await fetch(url, { headers: { accept: "application/json" }, signal: AbortSignal.timeout(3500), redirect: "error" });
      if (!response.ok) {
        await response.body?.cancel();
        continue;
      }
      const data = parseIpMetadata(await response.json(), ip, checkedAt);
      if (data) return data;
    } catch { /* The next provider or scheduled refresh can retry. */ }
  }
  return null;
}

function cacheStatement(env: Env, address: string, data: IpMetadata | null, next: number) {
  return env.DB.prepare(
    `INSERT INTO ip_metadata (address, details_json, refresh_after) VALUES (?, ?, ?)
     ON CONFLICT(address) DO UPDATE SET
        details_json = COALESCE(excluded.details_json, ip_metadata.details_json),
        refresh_after = excluded.refresh_after`,
  ).bind(address, data && JSON.stringify(data), next);
}

export async function refreshIpMetadata(env: Env, timestamp = now()) {
  const { results } = await env.DB.prepare(
    `SELECT DISTINCT n.address FROM nodes n LEFT JOIN ip_metadata m ON m.address = n.address
      WHERE n.address IS NOT NULL AND n.status != 'revoked'
        AND (m.refresh_after IS NULL OR m.refresh_after <= ?)
      ORDER BY m.refresh_after, n.address LIMIT ?`,
  ).bind(timestamp, limit).all<{ address: string }>();
  for (let offset = 0; offset < results.length; offset += concurrency) {
    const statements = await Promise.all(results.slice(offset, offset + concurrency).map(async ({ address }) => {
      const ip = publicIp(address);
      const data = ip ? await lookup(ip, timestamp) : null;
      return cacheStatement(env, address, data, timestamp + (ip && !data ? retry : week));
    }));
    await env.DB.batch(statements);
  }
}
