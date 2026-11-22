import type { ClusterNode, ClusterResponse, TunnelLane } from "../data/types";

interface Point { x: number; y: number }
export interface MappedNode extends Point { node: ClusterNode }
export interface MappedLane extends Point {
  lanes: TunnelLane[];
  bundleId?: string;
  source: ClusterNode;
  peer: ClusterNode;
  path: string;
}

interface RouteLanes {
  lanes: TunnelLane[];
  source: MappedNode;
  peer: MappedNode;
  bundleId?: string;
}

const desktop = { width: 944, cardWidth: 212, cardHeight: 142, gap: 23, inset: 24, top: 53 };
const portrait = { width: 358, cardWidth: 152, cardHeight: 142, gap: 14, inset: 16, top: 44 };
const labelStride = 37;
const denseLaneThreshold = 12;

export function laneBundleId(lane: TunnelLane) {
  return JSON.stringify([lane.source_node_id, lane.peer_node_id, lane.kind ?? "tunnel"]);
}

function port(node: MappedNode, fraction: number, isPortrait: boolean) {
  const domestic = node.node.node_class === "domestic";
  if (isPortrait) return { x: node.x + portrait.cardWidth * fraction, y: node.y + (domestic ? portrait.cardHeight : 0) };
  return { x: node.x + (domestic ? desktop.cardWidth : 0), y: node.y + desktop.cardHeight * fraction };
}

function curve(from: Point, via: Point, to: Point) {
  const entry = from.x + (via.x - from.x) * 0.56;
  const exit = to.x + (via.x - to.x) * 0.56;
  const split = from.x === to.x ? (from.y <= to.y ? 7 : -7) : 0;
  return `M ${from.x} ${from.y} C ${entry} ${from.y}, ${entry} ${via.y - split}, ${via.x} ${via.y - split} V ${via.y + split} C ${exit} ${via.y + split}, ${exit} ${to.y}, ${to.x} ${to.y}`;
}

function portraitCurve(from: Point, via: Point, to: Point) {
  const entry = from.y + (via.y - from.y) * 0.53;
  const exit = to.y + (via.y - to.y) * 0.53;
  const split = from.y === to.y ? (from.x <= to.x ? 24 : -24) : 0;
  return `M ${from.x} ${from.y} C ${from.x} ${entry}, ${via.x - split} ${entry}, ${via.x - split} ${via.y} H ${via.x + split} C ${via.x + split} ${exit}, ${to.x} ${exit}, ${to.x} ${to.y}`;
}

function bankPositions(items: ClusterNode[], isDomestic: boolean, isPortrait: boolean, width: number, height: number): MappedNode[] {
  const size = isPortrait ? portrait : desktop;
  return items.map((node, index) => ({
    node,
    x: isPortrait ? size.inset + index * (size.cardWidth + size.gap) : isDomestic ? size.inset : width - size.cardWidth - size.inset,
    y: isPortrait ? isDomestic ? size.top : height - size.cardHeight - size.inset : size.top + index * (size.cardHeight + size.gap),
  }));
}

function connect(lanes: TunnelLane[], nodes: MappedNode[], dense: boolean): RouteLanes[] {
  const byId = new Map(nodes.map((item) => [item.node.id, item]));
  const bundles = new Map<string, RouteLanes>();
  const routes: RouteLanes[] = [];
  for (const lane of lanes) {
    const source = byId.get(lane.source_node_id);
    const peer = byId.get(lane.peer_node_id);
    if (!source || !peer) continue;
    const key = dense ? laneBundleId(lane) : undefined;
    const previous = key === undefined ? undefined : bundles.get(key);
    if (previous) previous.lanes.push(lane);
    else {
      const route = { lanes: [lane], source, peer };
      routes.push(route);
      if (key !== undefined) bundles.set(key, route);
    }
  }
  for (const [key, route] of bundles) if (route.lanes.length > 1) route.bundleId = key;
  return routes;
}

function assignPorts(routes: RouteLanes[], isPortrait: boolean) {
  const axis = isPortrait ? "x" : "y";
  const coordinate = (route: RouteLanes, node: MappedNode) => (route.source === node ? route.peer : route.source)[axis];
  const byNode = new Map<MappedNode, RouteLanes[]>();
  for (const route of routes) {
    for (const node of new Set([route.source, route.peer])) {
      const edges = byNode.get(node) ?? [];
      edges.push(route);
      byNode.set(node, edges);
    }
  }
  const portAt = new Map<MappedNode, Map<RouteLanes, Point>>();
  for (const [node, edges] of byNode) {
    edges.sort((a, b) => coordinate(a, node) - coordinate(b, node));
    portAt.set(node, new Map(edges.map((edge, i) => [edge, port(node, 0.16 + 0.68 * (i + 1) / (edges.length + 1), isPortrait)])));
  }
  const end = (node: MappedNode, route: RouteLanes) => portAt.get(node)?.get(route) ?? node;
  return routes.map((edge) => ({ ...edge, from: end(edge.source, edge), to: end(edge.peer, edge) }));
}

function routeLayout(edges: ReturnType<typeof assignPorts>, width: number, height: number, isPortrait: boolean, dense: boolean): MappedLane[] {
  const middle = ({ from, to }: (typeof edges)[number]) => (from.y + to.y) / 2;
  const ordered = isPortrait || dense ? edges : [...edges].sort((first, second) => middle(first) - middle(second));
  let previous = desktop.top - labelStride;
  return ordered.map(({ from, to, lanes, bundleId, source, peer }, index) => {
    const available = height - 25 - (ordered.length - 1 - index) * labelStride;
    const x = !isPortrait ? width / 2 : dense ? (from.x + to.x) / 2 : Math.min(width / 2, portrait.width / 2) + (index % 2 === 0 ? -35 : 35);
    const center = (from.y + to.y) / 2;
    const y = dense ? isPortrait ? (portrait.top + height - portrait.inset) / 2 : center
      : isPortrait ? portrait.top + portrait.cardHeight + 26 + index * labelStride
      : Math.min(available, Math.max(previous + labelStride, center));
    previous = y;
    const path = (isPortrait ? portraitCurve : curve)(from, { x, y }, to);
    return { lanes, bundleId, source: source.node, peer: peer.node, x, y, path };
  });
}

export function topologyLayout(cluster: ClusterResponse, isPortrait: boolean, compact = false) {
  const domestic = cluster.nodes.filter((node) => node.node_class === "domestic");
  const foreign = cluster.nodes.filter((node) => node.node_class === "foreign");
  const rows = Math.max(domestic.length, foreign.length, 1);
  const ids = new Set(cluster.nodes.map((node) => node.id));
  const lanes = cluster.lanes.filter((lane) => ids.has(lane.source_node_id) && ids.has(lane.peer_node_id));
  const dense = lanes.length > denseLaneThreshold;
  // On dense maps, align the long server banks with the long side of the viewport.
  const stacked = dense ? !isPortrait : isPortrait;
  const laneSpace = dense ? 112 : lanes.length * labelStride;
  const width = stacked ? Math.max(portrait.width, rows * (portrait.cardWidth + portrait.gap) - portrait.gap + portrait.inset * 2)
    : dense ? 552 : compact ? 680 : desktop.width;
  const height = stacked ? portrait.top + 2 * portrait.cardHeight + Math.max(72, laneSpace + 36) + portrait.inset
    : Math.max(desktop.top + rows * (desktop.cardHeight + desktop.gap) + 7, desktop.top + laneSpace + 28);
  const footer = stacked && dense ? 18 : 0;
  const inside = bankPositions(domestic, true, stacked, width, height);
  const outside = bankPositions(foreign, false, stacked, width, height);
  const connected = connect(lanes, [...inside, ...outside], dense);
  const routes = routeLayout(assignPorts(connected, stacked), width, height, stacked, dense);
  return {
    width, height: height + footer, domestic: inside, foreign: outside, routes, dense, stacked,
    unmapped: cluster.lanes.length - lanes.length,
    domesticLabel: { x: (stacked ? portrait : desktop).inset, y: 17 },
    foreignLabel: stacked ? { x: portrait.inset, y: dense ? height - 7 : height - portrait.cardHeight - portrait.inset - 27 }
      : { x: width - desktop.cardWidth - desktop.inset, y: 17 },
  };
}
