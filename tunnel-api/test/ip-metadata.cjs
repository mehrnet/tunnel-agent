const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const { build } = require("esbuild");
const { getPlatformProxy } = require("wrangler");
const { parseIpMetadata, publicIp } = require("../src/ip-metadata.ts");

const timestamp = 1800000000;
const week = 7 * 24 * 3600;
const ipv4 = "1.1.1.1";
const ipv6 = "2606:4700:4700::1111";

function reply(ip) {
  return {
    ip,
    network: { cidr: "1.1.1.0/24", asns: ["AS13335", "AS64501"], origins: [{ asn: "AS13335", name: "CLOUDFLARENET" }] },
    allocation: { registry: "apnic", country_code: "US" },
    location: { country_code: "AU", region: "New South Wales", city: "Sydney" },
  };
}

test("parses the matching IP, prefers location over allocation, and retains all origin ASNs", () => {
  const data = parseIpMetadata(reply(ipv4), ipv4, timestamp);
  assert.equal(data.country_code, "AU");
  assert.equal(data.city, "Sydney");
  assert.equal(data.registry, "apnic");
  assert.equal(data.checked_at, timestamp);
  assert.deepEqual(data.origins.map(({ asn, name }) => [asn, name]), [["AS13335", "CLOUDFLARENET"], ["AS64501", null]]);
  assert.equal(parseIpMetadata({ ...reply(ipv4), location: null }, ipv4, timestamp).country_code, "US");
  assert.equal(parseIpMetadata(reply(ipv4), "8.8.8.8", timestamp), null);
  assert.equal(parseIpMetadata({ ip: ipv4 }, ipv4, timestamp), null);
});

test("public lookups accept IP literals without treating private addresses or hostnames as public IPs", () => {
  assert.equal(publicIp(ipv4), ipv4);
  assert.equal(publicIp("[2606:4700:4700:0:0:0:0:1111]"), ipv6);
  for (const address of ["127.0.0.1", "192.168.1.1", "10.0.0.1", "100.64.1.1", "256.1.1.1", "01.1.1.1",
    "1.1.1.1:443", "[::1]", "2001:db8::1", "fe80::1", "cloudflare.com"]) assert.equal(publicIp(address), null);
});

test("scheduled refresh deduplicates addresses, falls back on failure and serves the last good cache from D1", async (t) => {
  const platform = await getPlatformProxy({ persist: false });
  try {
    const { env } = platform;
    const built = await build({ stdin: { contents: [
      'export { refreshIpMetadata } from "./src/ip-metadata-refresh";',
      'export { observatoryNodes } from "./src/observatory-store";',
    ].join("\n"), resolveDir: process.cwd() }, bundle: true, format: "esm", write: false });
    const { refreshIpMetadata, observatoryNodes } = await import(`data:text/javascript;base64,${Buffer.from(built.outputFiles[0].contents).toString("base64")}`);
    for (const name of ["0001_initial", "0006_node_heartbeats", "0007_node_telemetry", "0018_ip_metadata"]) {
      const sql = readFileSync(`migrations/${name}.sql`, "utf8").replace(/^\s*--.*$/gm, "");
      await env.DB.batch(sql.split(";").map((s) => s.trim()).filter(Boolean).map((s) => env.DB.prepare(s)));
    }
    const run = (sql, ...args) => env.DB.prepare(sql).bind(...args).run();
    await run("INSERT INTO owners(id,telegram_user_id,created_at) VALUES('owner','123',0)");
    await run("INSERT INTO clusters(id,owner_id,name,created_at) VALUES('cluster','owner','oxygen',0)");
    for (const [id, address, status] of [
      ["a", ipv4, "enrolled"], ["b", ipv4, "pending"], ["c", ipv6, "pending"], ["lan", "192.168.0.1", "pending"], ["old", "8.8.8.8", "revoked"],
    ]) {
      await run("INSERT INTO nodes(id,owner_id,name,node_class,address,status,created_at) VALUES(?,'owner',?,'foreign',?,?,0)", id, id, address, status);
      await run("INSERT INTO cluster_nodes(cluster_id,node_id,role,created_at) VALUES('cluster',?,'foreign',0)", id);
    }
    const calls = [];
    let outage = false;
    t.mock.method(globalThis, "fetch", async (url) => {
      const ip = url.searchParams.get("query");
      calls.push([url.host, ip]);
      if (outage || (url.host === "bgp-lite.mehrnet.com" && ip === ipv6)) return new Response("Unavailable", { status: 503 });
      return Response.json(reply(ip));
    });
    await refreshIpMetadata(env, timestamp);
    assert.deepEqual(calls.sort(), [
      ["bgp-api.mehrnet.com", ipv6], ["bgp-lite.mehrnet.com", ipv4], ["bgp-lite.mehrnet.com", ipv6],
    ].sort());
    const read = async () => Object.fromEntries((await observatoryNodes(env, "owner", "cluster")).map((node) => [node.id, JSON.parse(node.ip_metadata_json)]));
    const cached = await read();
    assert.equal(cached.a.country_code, "AU");
    assert.deepEqual(cached.a, cached.b);
    assert.equal(cached.c.checked_at, timestamp);
    assert.equal(cached.lan, null);
    assert.equal(cached.old, null);
    await refreshIpMetadata(env, timestamp + 300);
    assert.equal(calls.length, 3);

    outage = true;
    await refreshIpMetadata(env, timestamp + week);
    assert.equal(calls.length, 7);
    assert.deepEqual(await read(), cached);
    await refreshIpMetadata(env, timestamp + week + 300);
    assert.equal(calls.length, 7);
    await run("UPDATE nodes SET address = '9.9.9.9' WHERE id = 'a'");
    assert.equal((await read()).a, null);
  } finally { await platform.dispose(); }
});
