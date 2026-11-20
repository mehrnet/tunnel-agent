import type { Env } from "./models";
import { retireFinishedProbe, writeProbe } from "./probe-report";
import type { TelemetryReport } from "./telemetry";
import { now } from "./validation";

export async function recordTelemetry(
  env: Env,
  nodeId: string,
  reporterNodeId: string,
  report: TelemetryReport,
): Promise<void> {
  const recordedAt = now();
  const statements = [
    env.DB.prepare(
      `INSERT INTO node_telemetry (
         node_id, reporter_node_id, sampled_at, recorded_at, cpu_milli_percent,
         memory_milli_percent, load_1_milli, rx_bytes_per_sec, tx_bytes_per_sec, agent_rss_bytes, transport_keys_json, probe_jobs
       ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(node_id) DO UPDATE SET
         reporter_node_id = excluded.reporter_node_id, sampled_at = excluded.sampled_at,
         recorded_at = excluded.recorded_at, cpu_milli_percent = excluded.cpu_milli_percent,
         memory_milli_percent = excluded.memory_milli_percent, load_1_milli = excluded.load_1_milli,
         rx_bytes_per_sec = excluded.rx_bytes_per_sec, tx_bytes_per_sec = excluded.tx_bytes_per_sec,
         agent_rss_bytes = excluded.agent_rss_bytes, transport_keys_json = excluded.transport_keys_json,
         probe_jobs = excluded.probe_jobs
       WHERE excluded.sampled_at > node_telemetry.sampled_at`,
    ).bind(
      nodeId,
      reporterNodeId,
      report.sampledAt,
      recordedAt,
      report.host.cpuMilliPercent,
      report.host.memoryMilliPercent,
      report.host.load1Milli,
      report.host.rxBytesPerSec,
      report.host.txBytesPerSec,
      report.host.agentRssBytes,
      report.transportKeys ? JSON.stringify(report.transportKeys) : null,
      Number(report.probeJobs),
    ),
    ...report.lanes.map((lane) => env.DB.prepare(
      `INSERT INTO lane_telemetry (
         node_id, lane_id, reporter_node_id, sampled_at, recorded_at, state, probe_ms
       ) VALUES (?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(node_id, lane_id) DO UPDATE SET
         reporter_node_id = excluded.reporter_node_id, sampled_at = excluded.sampled_at,
         recorded_at = excluded.recorded_at, state = excluded.state, probe_ms = excluded.probe_ms
       WHERE excluded.sampled_at > lane_telemetry.sampled_at`,
    ).bind(nodeId, lane.id, reporterNodeId, report.sampledAt, recordedAt, lane.state, lane.probeMs)),
  ];
  if (report.probe) statements.push(writeProbe(env, nodeId, report.sampledAt, report.probe));
  if (report.probeJobId) statements.push(retireFinishedProbe(env, nodeId, report.sampledAt, report.probeJobId));
  await env.DB.batch(statements);
}
