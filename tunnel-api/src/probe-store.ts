import { MAX_MANIFEST_BYTES } from "./documents";
import type { Env } from "./models";
import { PROBE_ID, PROBE_PERIOD, type ProbeProgress } from "./probe-report";
import { validTunnelHost } from "./tunnel-spec";
import { getIntent } from "./tunnel-store";
import { now } from "./validation";

interface JobSpec {
  id: string;
  starts_at: number;
  ends_at: number;
  port: number;
  hostname: string;
  path: string;
  expected_status: number;
  download: boolean;
}

interface JobRow {
  id: string;
  node_id: string;
  lane_id: string;
  starts_at: number;
  ends_at: number;
  spec_json: string;
  report_json: string | null;
  reported_at: number | null;
  cancelled_at: number | null;
}

interface Document {
  revision: number;
  manifest_json: string;
}

const FINAL_REPORT_GRACE = 120;
const pending = "cancelled_at IS NULL AND (ends_at>? OR (ends_at>? AND coalesce(reported_at,0)<ends_at+12))";

export async function listProbes(env: Env, cluster: string) {
  const { results } = await env.DB.prepare(
    `SELECT id,node_id,lane_id,starts_at,ends_at,spec_json,report_json,reported_at,cancelled_at
     FROM probe_jobs WHERE cluster_id=? ORDER BY starts_at DESC,id DESC LIMIT 25`,
  ).bind(cluster).all<JobRow>();
  return results.map(({ spec_json, report_json, ...job }) => ({
    ...job, target: JSON.parse(spec_json) as JobSpec, report: report_json ? JSON.parse(report_json) as ProbeProgress : null,
  }));
}

function probeSpec(body: Record<string, unknown>, port: number, start: number): JobSpec {
  const { duration, hostname, path, expected_status: status, download } = body;
  if (typeof duration !== "number" || duration < 600 || duration > 86400 || duration % PROBE_PERIOD !== 0) {
    throw new Error("Choose a duration from 10 minutes to 24 hours");
  }
  if (typeof hostname !== "string" || !hostname.includes(".") || !/[a-z]/i.test(hostname) || !validTunnelHost(hostname)) {
    throw new Error("Enter the destination TLS hostname");
  }
  if (typeof path !== "string" || !/^\/[a-z0-9/._~-]{0,159}$/i.test(path)) throw new Error("Invalid HTTPS path");
  if (typeof status !== "number" || !Number.isInteger(status) || status < 200 || status > 599 ||
      typeof download !== "boolean" || (download && status !== 200 && status !== 206)) {
    throw new Error("Select a valid expected status (200 or 206 for download probes)");
  }
  return { id: crypto.randomUUID(), starts_at: start, ends_at: start + duration, port,
    hostname, path, expected_status: status, download };
}

async function probeDocument(env: Env, id: string, timestamp: number): Promise<Document> {
  const doc = await env.DB.prepare(
    `SELECT d.revision,d.manifest_json FROM node_documents d
     JOIN nodes n ON n.id=d.node_id JOIN node_telemetry t ON t.node_id=n.id
     LEFT JOIN node_telemetry relay ON relay.node_id=t.reporter_node_id
     WHERE n.id=? AND n.status='enrolled' AND t.probe_jobs=1 AND t.sampled_at>=?
       AND (n.node_class='foreign' OR (relay.probe_jobs=1 AND relay.sampled_at>=?))`,
  ).bind(id, timestamp - 300, timestamp - 300).first<Document>();
  if (!doc) throw new Error("The listener and its management relay must report probe job support");
  return doc;
}

function encodeDocument(doc: Document, spec?: JobSpec) {
  const value = JSON.parse(doc.manifest_json) as { version: number; probe_job?: JobSpec };
  if (value.version !== 3) throw new Error("Probe jobs require a version 3 document");
  if (spec) value.probe_job = spec;
  else delete value.probe_job;
  const encoded = JSON.stringify(value);
  if (new TextEncoder().encode(encoded).byteLength > MAX_MANIFEST_BYTES) throw new Error("Server document is too large");
  return encoded;
}

function activeWithin(env: Env, node: string, timestamp: number) {
  return env.DB.prepare(`SELECT 1 FROM probe_jobs WHERE node_id=? AND ${pending}`)
    .bind(node, timestamp, timestamp - FINAL_REPORT_GRACE).first();
}

export async function startProbe(env: Env, cluster: string, body: Record<string, unknown>) {
  const laneId = body.lane_id;
  if (typeof laneId !== "string" || !PROBE_ID.test(laneId)) throw new Error("Select a managed lane");
  const lane = await getIntent(env, cluster, laneId);
  if (!lane?.enabled || lane.listener_type !== "tcp") throw new Error("Select an enabled TCP lane");
  const timestamp = now();
  const node = lane.listener_node_id;
  const spec = probeSpec(body, lane.listener_port, timestamp);
  const doc = await probeDocument(env, node, timestamp);
  if (await activeWithin(env, node, timestamp)) throw new Error("This listener has an active probe or is waiting for its final report");
  const [written] = await env.DB.batch([
    env.DB.prepare(
      `UPDATE node_documents SET manifest_json=?,revision=revision+1,updated_at=?
       WHERE node_id=? AND revision=? AND EXISTS (
         SELECT 1 FROM tunnel_intents i JOIN cluster_nodes m ON m.node_id=i.listener_node_id AND m.cluster_id=i.cluster_id
         JOIN nodes n ON n.id=m.node_id
         WHERE i.id=? AND i.revision=? AND i.enabled=1 AND i.listener_bind='127.0.0.1'
           AND n.id=node_documents.node_id AND n.status='enrolled'
       ) AND NOT EXISTS (
         SELECT 1 FROM probe_jobs p WHERE p.node_id=node_documents.node_id AND ${pending}
       )`,
    ).bind(encodeDocument(doc, spec), timestamp, node, doc.revision, laneId, lane.revision, timestamp, timestamp - FINAL_REPORT_GRACE),
    env.DB.prepare(
      `INSERT INTO probe_jobs (id,cluster_id,node_id,lane_id,starts_at,ends_at,spec_json)
        VALUES(CASE WHEN changes()=1 THEN ? ELSE NULL END,?,?,?,?,?,?)`,
    ).bind(spec.id, cluster, node, laneId, spec.starts_at, spec.ends_at, JSON.stringify(spec)),
  ]);
  if (written.meta.changes !== 1) throw new Error("Server configuration changed; reload and retry");
  return { id: spec.id };
}

export async function cancelProbe(env: Env, cluster: string, value: unknown) {
  if (typeof value !== "string" || !PROBE_ID.test(value)) throw new Error("Invalid job");
  const timestamp = now();
  const row = await env.DB.prepare(
    "SELECT p.node_id,d.revision,d.manifest_json FROM probe_jobs p JOIN node_documents d ON d.node_id=p.node_id WHERE p.id=? AND p.cluster_id=? AND p.cancelled_at IS NULL AND p.ends_at>?",
  ).bind(value, cluster, timestamp).first<Document & { node_id: string }>();
  if (!row) throw new Error("Job is no longer running");
  const [written] = await env.DB.batch([
    env.DB.prepare(
      `UPDATE node_documents SET manifest_json=?,revision=revision+1,updated_at=?
       WHERE node_id=? AND revision=? AND json_extract(manifest_json,'$.probe_job.id')=?`,
    ).bind(encodeDocument(row), timestamp, row.node_id, row.revision, value),
    env.DB.prepare("UPDATE probe_jobs SET cancelled_at=? WHERE id=? AND cancelled_at IS NULL AND changes()=1")
      .bind(timestamp, value),
  ]);
  if (written.meta.changes !== 1) throw new Error("Server configuration changed; reload and retry");
  return { cancelled: true };
}
