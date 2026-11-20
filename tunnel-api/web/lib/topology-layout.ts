import type { ClusterNode, ClusterResponse, TunnelLane } from "../data/types";

interface Point { x: number; y: number }
export interface MappedNode extends Point { node: ClusterNode }
export interface MappedLane extends Point {
  lane: TunnelLane;
  source: ClusterNode;
  peer: ClusterNode;
  path: string;
}

const desktop = { width: 944, cardWidth: 212, cardHeight: 142, gap: 23, inset: 24, top: 53 };
const portrait = { width: 358, cardWidth: 152, cardHeight: 142, gap: 14, inset: 16, top: 44 };
const labelStride = 37;

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

function connect(lanes: TunnelLane[], nodes: MappedNode[], isPortrait: boolean) {
  const byId = new Map(nodes.map((item) => [item.node.id, item]));
  const connected = lanes.flatMap((lane) => {
    const source = byId.get(lane.source_node_id);
    const peer = byId.get(lane.peer_node_id);
    return source && peer ? [{ lane, source, peer }] : [];
  });
  const totals = new Map<MappedNode, number>();
  const assigned = new Map<MappedNode, number>();
  for (const { source, peer } of connected) {
    for (const node of [source, peer]) totals.set(node, (totals.get(node) ?? 0) + 1);
  }
  const nextPort = (node: MappedNode) => {
    const next = (assigned.get(node) ?? 0) + 1;
    assigned.set(node, next);
    return port(node, 0.16 + 0.68 * next / ((totals.get(node) ?? 0) + 1), isPortrait);
  };
  return connected.map((edge) => ({ ...edge, from: nextPort(edge.source), to: nextPort(edge.peer) }));
}

function routeLayout(edges: ReturnType<typeof connect>, width: number, height: number, isPortrait: boolean): MappedLane[] {
  const middle = ({ from, to }: (typeof edges)[number]) => (from.y + to.y) / 2;
  const ordered = isPortrait ? edges : [...edges].sort((first, second) => middle(first) - middle(second));
  let previous = desktop.top - labelStride;
  return ordered.map((edge, index) => {
    const available = height - 25 - (ordered.length - 1 - index) * labelStride;
    const x = isPortrait ? Math.min(width / 2, portrait.width / 2) + (index % 2 === 0 ? -35 : 35) : width / 2;
    const y = isPortrait ? portrait.top + portrait.cardHeight + 26 + index * labelStride
      : Math.min(available, Math.max(previous + labelStride, middle(edge)));
    previous = y;
    const path = (isPortrait ? portraitCurve : curve)(edge.from, { x, y }, edge.to);
    return { lane: edge.lane, source: edge.source.node, peer: edge.peer.node, x, y, path };
  });
}

export function topologyLayout(cluster: ClusterResponse, isPortrait: boolean, compact = false) {
  const domestic = cluster.nodes.filter((node) => node.node_class === "domestic");
  const foreign = cluster.nodes.filter((node) => node.node_class === "foreign");
  const rows = Math.max(domestic.length, foreign.length, 1);
  const ids = new Set(cluster.nodes.map((node) => node.id));
  const lanes = cluster.lanes.filter((lane) => ids.has(lane.source_node_id) && ids.has(lane.peer_node_id));
  const laneSpace = lanes.length * labelStride;
  const width = isPortrait ? Math.max(portrait.width, rows * (portrait.cardWidth + portrait.gap) - portrait.gap + portrait.inset * 2)
    : compact ? 680 : desktop.width;
  const height = isPortrait ? portrait.top + 2 * portrait.cardHeight + Math.max(72, laneSpace + 36) + portrait.inset
    : Math.max(desktop.top + rows * (desktop.cardHeight + desktop.gap) + 7, desktop.top + laneSpace + 28);
  const inside = bankPositions(domestic, true, isPortrait, width, height);
  const outside = bankPositions(foreign, false, isPortrait, width, height);
  const routes = routeLayout(connect(lanes, [...inside, ...outside], isPortrait), width, height, isPortrait);
  return {
    width, height, domestic: inside, foreign: outside, routes,
    unmapped: cluster.lanes.length - routes.length,
    domesticLabel: { x: (isPortrait ? portrait : desktop).inset, y: 17 },
    foreignLabel: isPortrait ? { x: portrait.inset, y: height - portrait.cardHeight - portrait.inset - 27 }
      : { x: width - desktop.cardWidth - desktop.inset, y: 17 },
  };
}
