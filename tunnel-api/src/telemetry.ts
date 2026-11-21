import { error, json, parseJson } from "./http";
import type { Env } from "./models";
import { authorizeNode } from "./nodes";
import { foreignCanObserveDomestic } from "./heartbeats-store";
import { nodeById } from "./store";
import { parseProbe, PROBE_ID, type ProbeProgress } from "./probe-report";
import { recordTelemetry } from "./telemetry-store";
import { parseTransportKeys, type TransportKeys } from "./transport-keys";
import { now } from "./validation";

const MAX_LANES = 64;
const MAX_REPORTS = 64;
const MAX_FUTURE_SECONDS = 300;
const MAX_RATE = 10_000_000_000;
const NODE_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const LANE_ID = /^[A-Za-z0-9]{1,128}$/;

type LaneState = "healthy" | "degraded" | "unknown";

export interface TelemetryReport {
  sampledAt: number;
  host: {
    cpuMilliPercent: number;
    memoryMilliPercent: number;
    load1Milli: number;
    rxBytesPerSec: number;
    txBytesPerSec: number;
    agentRssBytes: number;
  };
  lanes: Array<{ id: string; state: LaneState; probeMs: number | null }>;
  transportKeys: TransportKeys | null;
  probeJobs: boolean;
  probe: ProbeProgress | null;
  probeJobId: string | null;
}

export async function handleTelemetry(request: Request, env: Env, nodeId: string): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  if (!await authorizeNode(request, env, nodeId)) return error("unauthorized", 401);
  const report = parseTelemetry(await parseJson(request));
  if (!report || report.sampledAt > now() + MAX_FUTURE_SECONDS) return error("invalid telemetry report", 400);
  await recordTelemetry(env, nodeId, nodeId, report);
  return json({ accepted: 1 });
}

export async function handleRelayTelemetry(request: Request, env: Env, foreignNodeId: string): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  if (!await authorizeNode(request, env, foreignNodeId)) return error("unauthorized", 401);
  const foreign = await nodeById(env, foreignNodeId);
  if (!foreign || foreign.node_class !== "foreign" || foreign.status !== "enrolled") {
    return error("only enrolled foreign relays may report telemetry", 403);
  }
  const reports = parseRelayReports(await parseJson(request));
  if (!reports || reports.some(({ report }) => report.sampledAt > now() + MAX_FUTURE_SECONDS)) {
    return error("invalid relay telemetry reports", 400);
  }
  const permitted = await Promise.all(reports.map(({ nodeId }) => foreignCanObserveDomestic(env, foreignNodeId, nodeId)));
  if (permitted.some((allowed) => !allowed)) return error("telemetry target is outside this relay's clusters", 403);
  await Promise.all(reports.map(({ nodeId, report }) => recordTelemetry(env, nodeId, foreignNodeId, report)));
  return json({ accepted: reports.length });
}

function parseRelayReports(body: Record<string, unknown> | null): Array<{ nodeId: string; report: TelemetryReport }> | null {
  const values = body?.reports;
  if (!Array.isArray(values) || values.length < 1 || values.length > MAX_REPORTS) return null;
  const reports = values.map((value) => {
    if (!isRecord(value) || typeof value.node_id !== "string" || !NODE_ID.test(value.node_id)) return null;
    const report = parseTelemetry(isRecord(value.telemetry) ? value.telemetry : null);
    return report ? { nodeId: value.node_id, report } : null;
  });
  return reports.some((report) => report === null) ? null : reports as Array<{ nodeId: string; report: TelemetryReport }>;
}

function parseTelemetry(body: Record<string, unknown> | null): TelemetryReport | null {
  const host = isRecord(body?.host) ? body.host : null;
  if (!host || !safeInteger(body?.sampled_at, 1, Number.MAX_SAFE_INTEGER)) return null;
  const fields: Array<[unknown, number]> = [
    [host.cpu_milli_percent, 100000], [host.memory_milli_percent, 100000], [host.load_1_milli, 1_000_000],
    [host.rx_bytes_per_sec, MAX_RATE], [host.tx_bytes_per_sec, MAX_RATE], [host.agent_rss_bytes, 1_000_000_000_000],
  ];
  if (fields.some(([value, limit]) => !safeInteger(value, 0, limit))) return null;
  const lanes = parseLanes(body?.lanes);
  if (!lanes) return null;
  const transportKeys = parseTransportKeys(body?.transport_keys);
  if (body?.transport_keys !== undefined && !transportKeys) return null;
  const probe = parseProbe(body?.probe);
  if (body?.probe !== undefined && !probe) return null;
  const probeJobId = body?.probe_job_id ?? null;
  if (probeJobId !== null && (typeof probeJobId !== "string" || !PROBE_ID.test(probeJobId))) return null;
  if (probe && probe.id !== probeJobId) return null;
  return {
    sampledAt: body.sampled_at as number,
    host: {
      cpuMilliPercent: host.cpu_milli_percent as number,
      memoryMilliPercent: host.memory_milli_percent as number,
      load1Milli: host.load_1_milli as number,
      rxBytesPerSec: host.rx_bytes_per_sec as number,
      txBytesPerSec: host.tx_bytes_per_sec as number,
      agentRssBytes: host.agent_rss_bytes as number,
    },
    lanes, transportKeys, probe, probeJobId, probeJobs: body?.probe_jobs === true,
  };
}

function parseLanes(value: unknown): TelemetryReport["lanes"] | null {
  if (!Array.isArray(value) || value.length > MAX_LANES) return null;
  const lanes = value.map((entry) => {
    if (!isRecord(entry) || typeof entry.id !== "string" || !LANE_ID.test(entry.id)) return null;
    if (!validState(entry.state) || !(entry.probe_ms === null || safeInteger(entry.probe_ms, 0, 60000))) return null;
    return { id: entry.id, state: entry.state, probeMs: entry.probe_ms };
  });
  return lanes.some((lane) => lane === null) ? null : lanes as TelemetryReport["lanes"];
}

function validState(value: unknown): value is LaneState {
  return value === "healthy" || value === "degraded" || value === "unknown";
}

function safeInteger(value: unknown, minimum: number, maximum: number): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= minimum && value <= maximum;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
