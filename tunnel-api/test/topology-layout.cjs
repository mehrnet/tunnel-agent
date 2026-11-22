const assert = require("node:assert/strict");
const test = require("node:test");
const { topologyLayout } = require("../web/lib/topology-layout.ts");

const node = (id, node_class) => ({ id, node_class });
const lane = (id, source_node_id, peer_node_id) => ({ id, source_node_id, peer_node_id });

function points(path) {
  return path.match(/-?\d+(?:\.\d+)?/g).map(Number);
}

test("same-bank lanes connect two actual card edges", () => {
  const nodes = [node("de-1", "foreign"), node("de-2", "foreign"), node("ir-1", "domestic")];
  const lanes = [lane("same", "de-1", "de-2"), lane("cross", "de-1", "ir-1")];
  const graph = topologyLayout({ nodes, lanes }, false);
  const same = graph.routes.find((route) => route.lanes[0].id === "same");
  const [fromX, fromY] = points(same.path);
  const [toX, toY] = points(same.path).slice(-2);
  assert.equal(fromX, graph.foreign[0].x);
  assert.equal(toX, graph.foreign[1].x);
  assert.ok(fromY >= graph.foreign[0].y && fromY < graph.foreign[1].y);
  assert.ok(toY >= graph.foreign[1].y && toY < graph.height);
});

test("parallel lanes have separate routes and missing endpoints remain counted", () => {
  const nodes = [node("de-1", "foreign"), node("ir-1", "domestic")];
  const lanes = [lane("primary", "de-1", "ir-1"), lane("fallback", "de-1", "ir-1"), lane("unmapped", "de-2", "ir-1")];
  for (const orientation of [false, true]) {
    const graph = topologyLayout({ nodes, lanes }, orientation);
    assert.equal(graph.unmapped, 1);
    assert.equal(graph.routes.length, 2);
    assert.notEqual(graph.routes[0].path, graph.routes[1].path);
    for (const route of graph.routes) {
      assert.ok(route.y > 0 && route.y < graph.height);
      assert.ok(route.x > 0 && route.x < graph.width);
    }
  }
});

test("dense maps bundle only matching directions and lane kinds without growing the canvas", () => {
  const domestic = Array.from({ length: 6 }, (_, i) => node(`ir-${i}`, "domestic"));
  const foreign = Array.from({ length: 6 }, (_, i) => node(`out-${i}`, "foreign"));
  const nodes = [...domestic, ...foreign];
  const lanes = domestic.slice(0, 4).flatMap((source) => foreign.slice(0, 4).flatMap((peer) =>
    Array.from({ length: 13 }, (_, i) => lane(`${source.id}-${peer.id}-${i}`, source.id, peer.id))));
  const reversed = lane("reverse", foreign[0].id, domestic[0].id);
  const management = { ...lanes[0], id: "management", kind: "management" };
  const unmapped = lane("missing", "absent", foreign[0].id);

  for (const portrait of [false, true]) {
    const graph = topologyLayout({ nodes, lanes: [...lanes, management, reversed, unmapped] }, portrait, true);
    const doubled = topologyLayout({ nodes, lanes: [...lanes, ...lanes] }, portrait, true);
    assert.equal(graph.height, doubled.height);
    assert.equal(graph.width, doubled.width);
    assert.ok(graph.height < 1100);
    assert.equal(graph.stacked, !portrait);
    assert.equal(graph.width > graph.height, !portrait);
    assert.equal(graph.unmapped, 1);
    assert.equal(graph.routes.length, 18);
    assert.equal(graph.routes.flatMap((route) => route.lanes).length, 210);
    const bundle = graph.routes.find((route) => route.lanes.includes(lanes[0]));
    assert.ok(bundle.bundleId);
    assert.equal(bundle.lanes.length, 13);
    for (const route of graph.routes) {
      assert.ok(route.x > 0 && route.x < graph.width);
      assert.ok(route.y > 0 && route.y < graph.height);
      const [fromX, fromY] = points(route.path);
      const [toX, toY] = points(route.path).slice(-2);
      assert.ok(fromX >= 0 && fromX <= graph.width && toX >= 0 && toX <= graph.width);
      assert.ok(fromY >= 0 && fromY <= graph.height && toY >= 0 && toY <= graph.height);
    }
  }
});
