const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync, readdirSync, writeFileSync, mkdirSync } = require("node:fs");
const { createHash, createHmac, randomUUID } = require("node:crypto");
const { join } = require("node:path");
const { getPlatformProxy } = require("wrangler");
const { build } = require("esbuild");
const { verifyIngress } = require("./ingress-owner.cjs");
const { verifyProbes } = require("./probe-owner.cjs");

const bot = "integration-test-token";

async function verifyLaneProbe(env, worker, readCluster, listener, id) {
  const token = randomUUID();
  const hash = value => createHash("sha256").update(value).digest("hex");
  await env.DB.prepare("UPDATE node_credentials SET secret_hash=? WHERE node_id=?").bind(hash(token), listener).run();
  const sampledAt = Math.floor(Date.now() / 1000);
  const response = await worker.fetch(new Request(`https://test.invalid/api/v1/nodes/${listener}/telemetry`, {
    method: "POST", headers: { authorization: `Bearer ${listener}.${token}` },
    body: JSON.stringify({
      sampled_at: sampledAt,
      host: {
        cpu_milli_percent: 1200, memory_milli_percent: 42000, load_1_milli: 10,
        rx_bytes_per_sec: 12, tx_bytes_per_sec: 7, agent_rss_bytes: 4096,
      },
      lanes: [{ id: "tunnel" + hash(`owned-${id}`), state: "healthy", probe_ms: 0 }],
    }),
  }), env, {});
  assert.equal(response.status, 200, await response.text());
  const snapshot = await readCluster();
  const [lane] = snapshot.lanes;
  assert.equal(lane.state, "healthy");
  assert.equal(lane.probe_scope, "local_control");
  assert.equal(lane.probe_ms, 0);
  assert.equal(lane.observed_at, sampledAt);
  assert.equal(lane.fresh_until, sampledAt + 180);
  assert.equal(snapshot.nodes.find(node => node.id === listener).heartbeat_expires_at, sampledAt + 180);
  await env.DB.prepare("UPDATE lane_telemetry SET sampled_at=? WHERE node_id=?").bind(sampledAt - 1800, listener).run();
  const outdated = (await readCluster()).lanes[0];
  assert.equal(outdated.state, "unknown");
  assert.equal(outdated.probe_ms, null);
}

function telegram(user) {
  const params = new URLSearchParams({ auth_date: String(Math.floor(Date.now() / 1000)), user: JSON.stringify({ id: user }) });
  const key = createHmac("sha256", "WebAppData").update(bot).digest();
  params.set("hash", createHmac("sha256", key).update([...params].map(([k,v]) => k+"="+v).join("\n")).digest("hex"));
  return params.toString();
}

async function verifyReleaseScope(worker, env, call, cluster, otherCluster, scoped, other) {
  const { privateKey, publicKey } = await crypto.subtle.generateKey("Ed25519", true, ["sign", "verify"]);
  env.CAPSULE_SIGNING_PRIVATE_KEY = Buffer.from(await crypto.subtle.exportKey("pkcs8", privateKey)).toString("base64");
  const digest = "a".repeat(64);
  const run = (sql, ...args) => env.DB.prepare(sql).bind(...args).run();
  await run("INSERT INTO bundle_releases(version,architecture,sha256,inventory_sha256,public_url,active,created_at) VALUES('v1','amd64',?,?,'/base',1,0)", digest, digest);
  const track = id => "/clusters/"+id+"/release-track";
  assert.equal((await call(track(cluster),{track:"testing"},102)).status,404);
  assert.equal((await call(track(cluster),{track:"testing"})).status,200);
  // A legacy per-node override must never take priority over a release track.
  await run("INSERT INTO seed_canary_releases VALUES(?,'amd64','v2-canary',?,?)", scoped, digest, digest);
  const auth = new Map();
  for (const id of [scoped, other]) {
    const secret = randomUUID();
    const hash = Buffer.from(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(secret))).toString("hex");
    await run("INSERT INTO node_credentials VALUES(?,?,0,0)", id, hash);
    auth.set(id, id+"."+secret);
  }
  async function gets(id, version) {
    const path = "https://test.invalid/api/v1/nodes/"+id+"/bundle-release?architecture=amd64";
    assert.equal((await worker.fetch(new Request(path), env, {})).status,401);
    const reply = await worker.fetch(new Request(path,{headers:{authorization:"Bearer "+auth.get(id)}}), env, {});
    assert.equal(reply.status,200);
    const { p, s } = JSON.parse((await reply.json()).envelope);
    const payload = Buffer.from(p,"base64url");
    assert(await crypto.subtle.verify("Ed25519",publicKey,Buffer.from(s,"base64url"),payload));
    assert.equal(JSON.parse(payload).b,version);
  }
  await gets(scoped, "v1");
  await gets(other, "v1");
  await run("INSERT INTO cluster_nodes VALUES(?,?,'foreign',0)",otherCluster,other);
  await gets(scoped, "v1");
  await run("DELETE FROM cluster_nodes WHERE cluster_id=?",otherCluster);
  assert.equal((await call(track(otherCluster),{track:"testing"})).status,200);
  await gets(scoped, "v1");
  const flags = (await (await call("/clusters")).json()).clusters.filter(c => c.release_track === "testing");
  assert.deepEqual(flags.map(c => c.id),[otherCluster]);

  env.BUNDLE_PUBLISH_TOKEN = "release-test-token";
  const activate = (track, bundles, version = "v0.5.1") => worker.fetch(new Request("https://test.invalid/api/v1/admin/bundles/activate",{
    method:"POST", headers:{authorization:"Bearer "+env.BUNDLE_PUBLISH_TOKEN},
    body:JSON.stringify({track,version,bundles}),
  }), env, {});
  const forVersion = version => [["amd64","x86_64"],["arm64","aarch64"]].map(([architecture,target])=>({
    architecture, sha256:digest, inventory_sha256:digest,
    public_url:`/bundles/${version}/tunnel-agent-${version}-${target}-unknown-linux-musl.tar.gz.parts`,
    domestic_urls:["https://mirror.example/bundle.parts"],
  }));
  const bundles = forVersion("v0.5.1");
  assert.equal((await activate("stable",bundles)).status,409);
  for (const version of ["v0.5.10", "v0.5.1-canary.2"]) {
    assert.equal((await activate("testing",forVersion(version),version)).status,400);
  }
  assert.equal((await activate("testing",bundles)).status,200);
  await gets(scoped,"v1");
  assert.equal((await call(track(cluster),{track:"testing"})).status,200);
  await gets(scoped,"v0.5.1");
  assert.equal((await activate("stable",[{...bundles[0],sha256:"b".repeat(64)},bundles[1]])).status,409);
  assert.equal((await call(track(cluster),{track:"stable"})).status,200);
  await gets(scoped,"v1");
  assert.equal((await activate("stable",bundles)).status,200);
  await gets(scoped,"v0.5.1");
  assert.equal((await activate("testing",bundles.map(bundle => ({ ...bundle, domestic_urls: [] })))).status,200);
  const mirrors = await env.DB.prepare("SELECT domestic_url FROM bundle_releases WHERE active=1 AND version='v0.5.1'").all();
  assert.deepEqual(mirrors.results.map(row => row.domestic_url), ["https://mirror.example/bundle.parts", "https://mirror.example/bundle.parts"]);
  assert.equal((await activate("testing",forVersion("v0.5.2"),"v0.5.2")).status,200);
  await gets(scoped,"v0.5.1");
  assert.equal((await call(track(cluster),{track:"testing"})).status,200);
  await gets(scoped,"v0.5.2");
}

function keyFixture(byte) {
  const raw = Buffer.alloc(32, byte);
  const prefix = Buffer.from("0000000b7373682d6564323535313900000020", "hex");
  return { hpnssh: Buffer.concat([prefix, raw]).toString("base64"), rathole: raw.toString("base64") };
}

async function recordTransportKeys(worker, env, node, transport_keys) {
  const token = randomUUID();
  const hash = createHash("sha256").update(token).digest("hex");
  await env.DB.prepare("UPDATE node_credentials SET secret_hash=? WHERE node_id=?").bind(hash, node).run();
  const reply = await worker.fetch(new Request(`https://test.invalid/api/v1/nodes/${node}/telemetry`, {
    method: "POST", headers: { authorization: `Bearer ${node}.${token}` },
    body: JSON.stringify({
      sampled_at: Math.floor(Date.now() / 1000) + 1, transport_keys, lanes: [],
      host: { cpu_milli_percent: 0, memory_milli_percent: 0, load_1_milli: 0, rx_bytes_per_sec: 0, tx_bytes_per_sec: 0, agent_rss_bytes: 0 },
    }),
  }), env, {});
  assert.equal(reply.status, 200, await reply.text());
}

async function verifyManagedTransports(worker, env, call, path, config, docs) {
  const listener_key = keyFixture(1);
  const connector_key = keyFixture(2);
  const create = transport => call(path + "/tunnels", { ...config, transport, listener_type: "tcp" });
  assert.equal((await call(path + "/tunnels", { ...config, transport: "rathole", listener_type: "socks5" })).status, 409);
  for (const transport of ["hpnssh", "rathole"]) {
    const created = await create(transport);
    assert.equal(created.status, 201, await created.clone().text());
    const { id } = await created.json();
    const lane = path + "/tunnels/" + id;
    const action = (action, revision) => call(lane, { action, revision });
    if (transport === "hpnssh") {
      assert.equal((await action("enable", 1)).status, 409);
      assert((await docs()).results.every(row => JSON.parse(row.manifest_json).tunnels.length === 0));
      await recordTransportKeys(worker, env, config.listener_node_id, listener_key);
      await recordTransportKeys(worker, env, config.source_node_id, connector_key);
    }
    const enabled = await action("enable", 1);
    assert.equal(enabled.status, 200, await enabled.text());
    const paired = async () => (await docs()).results.map(row => JSON.parse(row.manifest_json).tunnels[0]);
    const pair = await paired();
    const server = pair.find(spec => spec.role === "server");
    const client = pair.find(spec => spec.role === "client");
    assert.equal(server.contract, "managed.reverse.v1");
    assert.equal(server.transport, transport);
    assert.equal(server.peer_key, connector_key[transport]);
    assert.equal(client.peer_key, listener_key[transport]);
    assert.equal(client.local_key, connector_key[transport]);
    assert.deepEqual(client.listener.target, { host: config.destination_host, port: config.destination_port });
    assert(!server.listener.target);
    assert.equal(server.generation, client.generation);
    assert(!JSON.stringify(await (await call(path)).json()).includes(server.generation));
    assert.equal((await action("restart", 2)).status, 200);
    assert((await paired()).every(spec => spec.generation !== server.generation));
    assert.equal((await action("disable", 3)).status, 200);
    assert((await docs()).results.every(row => JSON.parse(row.manifest_json).tunnels.length === 0));
    assert.equal((await action("remove", 4)).status, 200);
  }
}

test("worker creates and controls a paired lane without exposing its capability", async () => {
  const platform = await getPlatformProxy({ persist: false });
  try {
    const env = { ...platform.env, TELEGRAM_BOT_TOKEN: bot, MANAGED_TUNNEL_ACTIVATION: "0" };
    const bundle = await build({ entryPoints: ["src/worker.ts"], bundle: true, format: "esm", write: false });
    const { default: worker } = await import("data:text/javascript;base64," + Buffer.from(bundle.outputFiles[0].contents).toString("base64"));
    for (const name of readdirSync("migrations").filter(n => n.endsWith(".sql")).sort()) {
      const sql = readFileSync("migrations/" + name, "utf8").replace(/^\s*--.*$/gm, "");
      await env.DB.batch(sql.split(";").map(s => s.trim()).filter(Boolean).map(s => env.DB.prepare(s)));
    }
    const call = (path, body, user = 101) => worker.fetch(new Request("https://test.invalid/api/v1/web"+path, {
      method: body ? "POST" : "GET", headers: { "x-telegram-init-data": telegram(user) }, body: body && JSON.stringify(body),
    }), env, {});
    const { cluster } = await (await call("/clusters", { name: "isolated" })).json();
    const { cluster: otherCluster } = await (await call("/clusters", { name: "other" })).json();
    const path = "/clusters/"+cluster.id;
    async function node(name) {
      const reply = await call("/nodes", { name, cluster_id: cluster.id, address: "127.0.0.1", role: "foreign" });
      assert.equal(reply.status, 200);
      return (await reply.json()).node.id;
    }
    const source = await node("source");
    const listener = await node("listener");
    const docs = () => env.DB.prepare("SELECT revision,manifest_json FROM node_documents ORDER BY node_id").all();
    const fixture = process.env.TUNNEL_TEST_MANIFEST_DIR;
    async function recordPhase(phase) {
      if (!fixture) return;
      const rows = await env.DB.prepare("SELECT node_id,revision,manifest_json FROM node_documents").all();
      const pair = Object.fromEntries(rows.results.map(row => [row.node_id === source ? "client" : "server",
        { revision:row.revision, manifest:JSON.parse(row.manifest_json) }]));
      mkdirSync(fixture,{recursive:true});
      writeFileSync(join(fixture,phase+".json"),JSON.stringify(pair),{mode:0o600});
    }
    const type = process.env.TUNNEL_TEST_LISTENER === "socks5" ? "socks5" : "tcp";
    const config = { source_node_id: source, listener_node_id: listener, listener_type:type, transport:"wstunnel",
      destination_host:"127.0.0.1", destination_port:31401, listener_port:31402, control_port:31403, tls_server_name:"localhost" };
    assert.equal((await call(path+"/tunnels", { ...config, tls_server_name: "" })).status,409);
    assert.equal((await call(path+"/tunnels", { ...config, tls_server_name: "127.0.0.1" })).status,409);
    assert.equal((await call(path+"/tunnels", config, 102)).status,404);
    const attempts = await Promise.all([call(path+"/tunnels", config), call(path+"/tunnels", config)]);
    assert.deepEqual(attempts.map(r => r.status).sort(),[201,409]);
    const created = await attempts.find(r => r.status === 201).json();
    await recordPhase("draft");
    const lane = path+"/tunnels/"+created.id;
    assert.equal((await call(path+"/tunnels", { ...config, listener_port:config.control_port, control_port:31404 })).status,409);
    assert.equal((await call(lane, { action:"enable", revision:1 })).status,409);
    assert.deepEqual((await docs()).results.map(row=>row.revision),[1,1]);
    env.MANAGED_TUNNEL_ACTIVATION = "1";
    assert.equal((await call(lane, { action:"enable", revision:1 })).status,409);
    await env.DB.prepare("UPDATE nodes SET status='enrolled'").run();
    await verifyReleaseScope(worker,env,call,cluster.id,otherCluster.id,source,listener);
    const enabled = await call(lane,{ action:"enable",revision:1 });
    assert.equal(enabled.status,200,await enabled.text());
    await recordPhase("enabled");
    const before = (await docs()).results.map(row=>JSON.parse(row.manifest_json));
    const key = before[0].tunnels[0].endpoint?.path_prefix ?? before[0].tunnels[0].path_prefix;
    assert.equal((await call(lane,{ action:"disable",revision:1 })).status,409);
    assert.equal((await call(lane,{ action:"restart",revision:2 })).status,200);
    await recordPhase("restarted");
    const snapshot = await (await call(path)).json();
    assert.equal(snapshot.lanes.length,1);
    assert.equal(snapshot.lanes[0].managed.enabled,true);
    assert.equal(snapshot.lanes[0].managed.tls_server_name,"localhost");
    assert.equal(snapshot.lanes[0].transport, "wstunnel");
    assert.equal(snapshot.lanes[0].direction, "reverse");
    assert.equal(snapshot.lanes[0].priority, null);
    assert.equal(snapshot.lanes[0].local_listener, `${type} 127.0.0.1:31402`);
    assert.equal(snapshot.lanes[0].state, "unknown");
    assert(!JSON.stringify(snapshot).includes(key));
    await verifyLaneProbe(env, worker, async () => (await call(path)).json(), listener, created.id);
    const changed = (await docs()).results.map(row=>JSON.parse(row.manifest_json));
    const client = changed.find(doc => doc.tunnels[0].endpoint);
    const server = changed.find(doc => doc.tunnels[0].path_prefix);
    assert.equal(client.tunnels[0].endpoint.url,"wss://127.0.0.1:31403");
    assert.equal(client.tunnels[0].endpoint.tls_sni,"localhost");
    assert.notEqual(client.tunnels[0].endpoint.path_prefix,key);
    assert.equal(client.tunnels[0].endpoint.path_prefix,server.tunnels[0].path_prefix);
    if (process.env.TUNNEL_TEST_MANIFEST_DIR) {
      mkdirSync(process.env.TUNNEL_TEST_MANIFEST_DIR,{ recursive:true });
      for (const [name, doc] of Object.entries({ client,server })) {
        writeFileSync(join(process.env.TUNNEL_TEST_MANIFEST_DIR,name+".json"),JSON.stringify(doc),{ mode:0o600 });
      }
    }
    const edited = await call(lane,{ action:"destination",revision:3,destination_host:"127.0.0.1",destination_port:31404 });
    assert.equal(edited.status,type === "tcp" ? 200 : 400);
    await recordPhase("edited");
    const revision = type === "tcp" ? 4 : 3;
    assert.equal((await call(lane,{ action:"remove",revision })).status,409);
    assert.equal((await call(lane,{ action:"disable",revision })).status,200);
    await recordPhase("disabled");
    assert.equal((await call(lane,{ action:"remove",revision:revision+1 })).status,200);
    await recordPhase("removed");
    assert((await docs()).results.every(row=>JSON.parse(row.manifest_json).tunnels.length===0));
    await verifyManagedTransports(worker, env, call, path, config, docs);
    await verifyIngress({ env, call, path, config, report: (keys) => recordTransportKeys(worker, env, listener, keys), keys: keyFixture(1) });
    await verifyProbes({ worker, env, call, path, config, cluster: cluster.id });
  } finally { await platform.dispose(); }
});
