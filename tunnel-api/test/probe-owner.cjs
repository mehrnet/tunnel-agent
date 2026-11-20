const assert = require("node:assert/strict");
const { createHash, randomUUID } = require("node:crypto");

async function verifyProbes({ worker, env, call, path, config, cluster }) {
  const listener = config.listener_node_id;
  const relay = config.source_node_id;
  const run = (sql, ...args) => env.DB.prepare(sql).bind(...args).run();
  const readDoc = async () => JSON.parse(await env.DB.prepare(
    "SELECT manifest_json FROM node_documents WHERE node_id=?",
  ).bind(listener).first("manifest_json"));
  const epoch = Math.floor(Date.now() / 1000);
  const token = randomUUID();
  const transportKeys = await env.DB.prepare("SELECT transport_keys_json FROM node_telemetry WHERE node_id=?")
    .bind(relay).first("transport_keys_json");
  await run("UPDATE node_credentials SET secret_hash=? WHERE node_id=?", createHash("sha256").update(token).digest("hex"), relay);
  await run("UPDATE nodes SET node_class='domestic' WHERE id=?", listener);
  await run("UPDATE cluster_nodes SET role='domestic' WHERE node_id=? AND cluster_id=?", listener, cluster);
  async function telemetry(body, direct = false) {
    const report = { sampled_at: epoch + 2, lanes: [], probe_jobs: true,
      host: { cpu_milli_percent: 0, memory_milli_percent: 0, load_1_milli: 0,
        rx_bytes_per_sec: 0, tx_bytes_per_sec: 0, agent_rss_bytes: 0 }, ...body };
    return worker.fetch(new Request(`https://test.invalid/api/v1/nodes/${relay}/${direct ? "telemetry" : "relay-telemetry"}`, {
      method: "POST", headers: { authorization: `Bearer ${relay}.${token}` },
      body: JSON.stringify(direct ? report : { reports: [{ node_id: listener, telemetry: report }] }),
    }), env, {});
  }

  const listenerPort = config.listener_port + 50;
  const lane = await call(path + "/tunnels", { ...config, listener_type: "tcp", transport: "wstunnel",
    listener_port: listenerPort, control_port: config.control_port + 50 });
  assert.equal(lane.status, 201, await lane.clone().text());
  const { id } = await lane.json();
  const update = (action, revision) => call(path + "/tunnels/" + id, { action, revision });
  assert.equal((await update("enable", 1)).status, 200);
  const endpoint = path + "/probes";
  const start = { action: "start", lane_id: id, duration: 600, hostname: "origin.example.com", path: "/", expected_status: 404, download: false };
  assert.equal((await call(endpoint, start, 102)).status, 404);
  assert.equal((await call(endpoint, start)).status, 409);
  assert.equal((await telemetry({})).status, 200);
  assert.equal((await call(endpoint, start)).status, 409);
  assert.equal((await telemetry({ transport_keys: JSON.parse(transportKeys) }, true)).status, 200);
  assert.equal((await call(endpoint, { ...start, hostname: "127.0.0.1" })).status, 409);
  const requests = await Promise.all([call(endpoint, start), call(endpoint, start)]);
  assert.deepEqual(requests.map((item) => item.status).sort(), [201, 409]);
  const job = await requests.find((item) => item.status === 201).json();
  assert.equal((await readDoc()).probe_job.id, job.id);
  assert.equal((await readDoc()).probe_job.port, listenerPort);
  assert.equal((await update("disable", 2)).status, 409);
  const sample = { id: job.id, sequence: 2, passed: 1, failed: 0, streak: 0, longest_streak: 0,
    latency_total_ms: 123, latency_max_ms: 123, bytes: 0, download_ms: 0, last_error: null };
  const stamp = epoch + 62;
  assert.equal((await telemetry({ sampled_at: stamp, probe_job_id: job.id, probe: sample })).status, 200);
  assert.equal((await telemetry({ sampled_at: stamp + 1, probe_job_id: job.id,
    probe: { ...sample, sequence: 1 } })).status, 200);
  assert.equal((await telemetry({ probe_job_id: job.id, probe: { ...sample, id: randomUUID() } })).status, 400);
  const snapshot = (await (await call(path)).json()).probe_jobs.find((item) => item.id === job.id);
  assert.deepEqual(snapshot.report, sample);
  assert.equal((await call(endpoint, { action: "cancel", id: job.id })).status, 200);
  assert.equal((await readDoc()).probe_job, undefined);
  const next = await call(endpoint, { ...start, duration: 86_400 });
  assert.equal(next.status, 201, await next.clone().text());
  const nextJob = await next.json();
  await run("UPDATE node_documents SET manifest_json=json_set(manifest_json,'$.probe_job.ends_at',?) WHERE node_id=?", epoch - 20, listener);
  assert.equal((await telemetry({ sampled_at: stamp + 2, probe_job_id: nextJob.id })).status, 200);
  assert.equal((await readDoc()).probe_job, undefined);
  await run("UPDATE probe_jobs SET ends_at=starts_at+600,cancelled_at=? WHERE id=?", epoch, nextJob.id);
  assert.equal((await update("disable", 2)).status, 200);
  assert.equal((await update("remove", 3)).status, 200);
  await run("UPDATE nodes SET node_class='foreign' WHERE id=?", listener);
  await run("UPDATE cluster_nodes SET role='foreign' WHERE node_id=? AND cluster_id=?", listener, cluster);
  await run("DELETE FROM node_telemetry WHERE node_id=?", listener);
}

module.exports = { verifyProbes };
