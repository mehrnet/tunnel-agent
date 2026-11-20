import type { Env } from "./models";

export const PROBE_PERIOD = 30;
const PROBE_BYTES = 65_536;
export const PROBE_ID = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/;

export interface ProbeProgress {
  id: string;
  sequence: number;
  passed: number;
  failed: number;
  streak: number;
  longest_streak: number;
  latency_total_ms: number;
  latency_max_ms: number;
  bytes: number;
  download_ms: number;
  last_error: string | null;
}

export function parseProbe(value: unknown): ProbeProgress | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const item = value as ProbeProgress;
  if (typeof item.id !== "string" || !PROBE_ID.test(item.id)) return null;
  const bounds = { sequence: 2880, passed: 2880, failed: 2880, streak: 2880, longest_streak: 2880,
    latency_total_ms: 23_040_000, latency_max_ms: 8000, bytes: 2880 * PROBE_BYTES, download_ms: 23_040_000 };
  if (Object.entries(bounds).some(([key, max]) => !bounded(item[key as keyof typeof bounds], max))) return null;
  if (item.sequence < 1 || item.passed + item.failed < 1 || item.passed + item.failed > item.sequence ||
      item.streak > item.longest_streak || item.longest_streak > item.failed) return null;
  if (item.last_error !== null && !["timeout", "transport", "status", "short_body"].includes(item.last_error)) return null;
  if (item.bytes > item.passed * PROBE_BYTES || item.bytes % PROBE_BYTES !== 0) return null;
  if (item.latency_total_ms > item.passed * 8000 || item.latency_max_ms > item.latency_total_ms ||
      item.download_ms > item.passed * 8000 || (item.bytes > 0) !== (item.download_ms > 0)) return null;
  const { id, sequence, passed, failed, streak, longest_streak, latency_total_ms, latency_max_ms, bytes, download_ms, last_error } = item;
  return { id, sequence, passed, failed, streak, longest_streak, latency_total_ms, latency_max_ms, bytes, download_ms, last_error };
}

function bounded(value: number, max: number) {
  return Number.isSafeInteger(value) && value >= 0 && value <= max;
}

export function writeProbe(env: Env, node: string, sampledAt: number, report: ProbeProgress) {
  return env.DB.prepare(
    `UPDATE probe_jobs SET sequence=?, report_json=?, reported_at=?
     WHERE id=? AND node_id=? AND sequence<? AND starts_at<=?
       AND (cancelled_at IS NULL OR ?<=cancelled_at)
       AND ?<=min((ends_at-starts_at)/?, (? - starts_at)/? + 1)
       AND (?=0 OR json_extract(spec_json,'$.download')=1)`,
  ).bind(report.sequence, JSON.stringify(report), sampledAt, report.id, node, report.sequence, sampledAt,
    sampledAt, report.sequence, PROBE_PERIOD, sampledAt, PROBE_PERIOD, report.bytes);
}

export function retireFinishedProbe(env: Env, node: string, sampledAt: number, id: string) {
  return env.DB.prepare(
    `UPDATE node_documents SET manifest_json=json_remove(manifest_json,'$.probe_job'),
       revision=revision+1, updated_at=unixepoch()
     WHERE node_id=? AND json_extract(manifest_json,'$.probe_job.id')=?
       AND json_extract(manifest_json,'$.probe_job.ends_at')+12<=?`,
  ).bind(node, id, sampledAt);
}
