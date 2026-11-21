const assert = require("node:assert/strict");

async function createLane(call, path, config, transport, offset) {
  const result = await call(path + "/tunnels", {
    ...config, listener_type: "tcp", transport,
    listener_port: config.listener_port + offset,
    control_port: config.control_port + offset,
  });
  assert.equal(result.status, 201, await result.clone().text());
  const { id } = await result.json();
  const enabled = await call(path + "/tunnels/" + id, { action: "enable", revision: 1 });
  assert.equal(enabled.status, 200, await enabled.text());
  return id;
}

async function verifyIngress({ env, call, path, config, report, keys }) {
  const listener = config.listener_node_id;
  await env.DB.prepare("UPDATE nodes SET address='192.0.2.1' WHERE id=?").bind(listener).run();
  const doc = () => env.DB.prepare(
    "SELECT revision,manifest_json FROM node_documents WHERE node_id=?",
  ).bind(listener).first();
  const wstunnel = await createLane(call, path, config, "wstunnel", 0);
  const hpn = await createLane(call, path, config, "hpnssh", 10);
  const rathole = await createLane(call, path, config, "rathole", 20);
  const ingress = path + "/public-ingress";
  const request = { action: "set", node_id: listener, port: 8071,
    lane_id: hpn, fallback_lane_id: wstunnel, tls_server_name: "origin.example.com",
    revision: (await doc()).revision };
  assert.equal((await call(ingress, request)).status, 409);
  await report({ ...keys, kernel_ingress: true });
  assert.equal((await call(ingress, request, 102)).status, 404);
  assert.equal((await call(ingress, { ...request, port: config.listener_port })).status, 409);
  const result = await call(ingress, request);
  assert.equal(result.status, 200, await result.clone().text());
  const { revision } = await result.json();
  const initial = JSON.parse((await doc()).manifest_json);
  assert.equal(initial.public_ingress[0].primary, config.listener_port + 10);
  assert.equal(initial.public_ingress[0].fallback, config.listener_port);
  assert.equal((await call(ingress, { ...request, revision, lane_id: rathole })).status, 409);
  assert.equal((await call(path + "/tunnels/" + wstunnel, { action: "disable", revision: 2 })).status, 409);
  const flip = { ...request, revision, lane_id: wstunnel };
  const attempts = await Promise.all([call(ingress, flip), call(ingress, flip)]);
  assert.deepEqual(attempts.map((item) => item.status).sort(), [200, 409]);
  const after = JSON.parse((await doc()).manifest_json);
  assert.deepEqual(after.tunnels, initial.tunnels);
  assert.equal(after.public_ingress[0].lane_id, wstunnel);
  assert.equal(after.public_ingress[0].fallback_lane_id, hpn);
  const snapshot = await (await call(path)).json();
  assert.equal(snapshot.public_ingress[0].revision, revision + 1);
  assert.equal(snapshot.public_ingress[0].lane_id, wstunnel);
  assert.equal((await call(path + "/tunnels", { ...config,
    listener_type: "tcp", listener_port: 8071, control_port: 31442,
  })).status, 409);
}

async function verifyDualIngress({ env, call, path, config, report, keys }) {
  const listener = config.listener_node_id;
  const docs = () => env.DB.prepare("SELECT revision,manifest_json FROM node_documents WHERE node_id=?").bind(listener).first();
  const tunnels = path + "/tunnels";
  const ingress = path + "/public-ingress";
  const draft = await call(tunnels, { ...config, transport: "wstunnel", listener_type: "tcp", forward_udp: true,
    listener_port: 48000, control_port: 47000 });
  assert.equal(draft.status, 201, await draft.clone().text());
  const { id } = await draft.json();
  const enable = lane => call(tunnels + "/" + lane, { action: "enable", revision: 1 });
  assert.equal((await enable(id)).status, 409);
  assert(!JSON.parse((await docs()).manifest_json).tunnels.some(t => t.id === "owned-" + id));
  for (const node of [listener, config.source_node_id]) {
    await report(node, { ...keys, kernel_ingress: true, public_forward_v2: true });
  }
  const enabled = await enable(id);
  assert.equal(enabled.status, 200, await enabled.text());
  for (let i = 0; i < 20; i++) {
    const lane = i === 0 ? id : await createLane(call, path, {
      ...config, listener_port: 48000, control_port: 47000, forward_udp: true,
    }, "wstunnel", i);
    const put = await call(ingress, { action: "set", node_id: listener, port: 31000 + i,
      lane_id: lane, revision: (await docs()).revision });
    assert.equal(put.status, 200, await put.text());
  }
  const finalDoc = JSON.parse((await docs()).manifest_json);
  const dual = finalDoc.public_ingress.filter(route => route.forward_udp);
  assert.equal(dual.length, 20);
  assert.equal(finalDoc.tunnels.find(t => t.id === "owned-" + id).fixed_reverse_port, 48000);
  const source = await env.DB.prepare("SELECT manifest_json FROM node_documents WHERE node_id=?").bind(config.source_node_id).first();
  assert.equal(JSON.parse(source.manifest_json).tunnels.find(t => t.id === "owned-" + id).forward_udp, true);
  const remove = await call(ingress, { action: "remove", node_id: listener, port: 31000, revision: (await docs()).revision });
  assert.equal(remove.status, 200, await remove.text());
  assert.equal((await call(tunnels + "/" + id, { action: "disable", revision: 2 })).status, 200);
}

module.exports = { verifyIngress, verifyDualIngress };
