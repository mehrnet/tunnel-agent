const assert = require("node:assert/strict");
const { test } = require("node:test");
const { build } = require("esbuild");

test("parallel management edges merge across priorities and need both active ends", async () => {
  const buildResult = await build({
    entryPoints: [require.resolve("../src/observatory-lanes.ts")],
    bundle: true, format: "esm", platform: "node", write: false,
  });
  const { observatoryLanes } = await import(
    "data:text/javascript;base64," + Buffer.from(buildResult.outputFiles[0].text).toString("base64")
  );
  const createDoc = (nodeId, peerNodeId, role, maxActive) => ({
    node_id: nodeId, revision: 3, updated_at: 100,
    manifest_json: JSON.stringify({ agent: {
      max_active_lanes: maxActive,
      lanes: ["hpnssh", "wstunnel", "rathole"].map((transport, index) => ({
        id: nodeId + transport, transport, direction: "foreign_to_domestic",
        role, priority: index + (role === "client" ? 10 : 20),
        peer: { node_id: peerNodeId },
        listener: { bind: "127.0.0.1", port: 42010 + index },
      })),
    }}),
  });
  const docs = [createDoc("foreign", "domestic", "client", 3), createDoc("domestic", "foreign", "server", 2)];
  const sample = {
    node_id: "domestic", lane_id: "domesticwstunnel", state: "healthy",
    sampled_at: 110, probe_ms: 10, reporter_node_id: "foreign",
  };
  const edges = await observatoryLanes(docs, [sample], 120).lanes([], []);
  assert.deepEqual(edges.map(({ transport, selected }) => [transport, selected]), [
    ["hpnssh", true], ["wstunnel", true], ["rathole", false],
  ]);
  assert.equal(edges[1].source_node_id, "foreign");
  assert.equal(edges[1].state, "healthy");
  assert.equal(edges[1].probe_scope, "local_listener");
});
