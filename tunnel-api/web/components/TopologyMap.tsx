import { For, Show, createEffect, createSignal } from "solid-js";
import type { ClusterNode, ClusterResponse, TunnelLane } from "../data/types";
import { formatBytes, formatMoment, formatNumber, formatPercent, formatProbe, formatRate, isFresh, laneState, laneStatus, probeLabel } from "../lib/format";
import { StatIcon } from "./StatIcon";
import { createViewport } from "../lib/viewport";

interface TopologyMapProps {
  cluster: ClusterResponse;
  selectedKind: "lane" | "node" | undefined;
  selectedId: string | undefined;
  onNodeSelect: (id: string) => void;
  onLaneSelect: (id: string) => void;
}

function nodeStatus(node: ClusterNode) {
  if (node.status === "revoked") return "Revoked";
  if (node.status === "pending") return "Pending";
  return node.last_heartbeat_at === null ? "No report" : isFresh(node.heartbeat_expires_at) ? "Active" : "Stalled";
}

function savedOrientation() {
  try { return localStorage.getItem("tunnel-orientation") === "vertical"; } catch { return false; }
}
function laneClass(lane: TunnelLane, selected: boolean) {
  return `route-line route-line--${lane.kind ?? "tunnel"} route-line--${laneState(lane)}${selected ? " route-line--selected" : ""}`;
}

function nodeY(index: number) {
  return 90 + index * 196;
}

function NodeCard(props: { node: ClusterNode; selected: boolean; onSelect: () => void }) {
  const telemetry = () => props.node.telemetry;
  const classLabel = () => props.node.node_class === "foreign" ? "Foreign" : "Domestic";
  return (
    <button class="map-node" classList={{ "map-node--selected": props.selected }} type="button" aria-pressed={props.selected} onClick={() => props.onSelect()}>
      <span class="map-node__topline"><span>{classLabel()}</span><b>{nodeStatus(props.node)}</b></span>
      <strong>{props.node.name}</strong>
      <code>{props.node.address ?? "No address"}</code>
      <span class="map-node__heartbeat">Heartbeat: {formatMoment(props.node.last_heartbeat_at)}</span>
      <Show when={telemetry()} fallback={<span class="map-node__missing">No host telemetry</span>}>
        {(sample) => <div class="map-node__metrics">
          <span><em><StatIcon label="CPU" />CPU</em><b>{formatPercent(sample().cpu_percent)}</b></span>
          <span><em><StatIcon label="MEM" />MEM</em><b>{formatPercent(sample().memory_percent)}</b></span>
          <span><em><StatIcon label="RX" />RX</em><b>{formatRate(sample().rx_bytes_per_sec)}</b></span>
          <span><em><StatIcon label="TX" />TX</em><b>{formatRate(sample().tx_bytes_per_sec)}</b></span>
          <span><em><StatIcon label="LOAD" />LOAD</em><b>{formatNumber(sample().load_1, 2)}</b></span>
          <span><em><StatIcon label="RSS" />RSS</em><b>{formatBytes(sample().agent_rss_bytes)}</b></span>
        </div>}
      </Show>
    </button>
  );
}

function Route(props: { lane: TunnelLane; sourceY: number; peerY: number; labelY: number; reverse: boolean; selected: boolean; onSelect: () => void }) {
  const path = () => `M ${props.reverse ? 720 : 280} ${props.sourceY} Q 500 ${2 * props.labelY - (props.sourceY + props.peerY) / 2} ${props.reverse ? 280 : 720} ${props.peerY}`;
  const probe = () => formatProbe(props.lane.probe_ms, props.lane.fresh_until);
  const label = () => [props.lane.transport, props.lane.direction, props.lane.local_listener && `(${props.lane.local_listener})`].filter(Boolean).join(" ");
  const keySelect = (event: KeyboardEvent) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      props.onSelect();
    }
  };
  return (
    <g class={laneClass(props.lane, props.selected)} role="button" tabindex="0" aria-pressed={props.selected} aria-label={`Connection ${props.lane.transport} · ${laneStatus(props.lane)}`} onClick={() => props.onSelect()} onKeyDown={keySelect}>
      <path class="route-line__hit" d={path()} fill="none" />
      <path class="route-line__track" d={path()} marker-end={`url(#arrow-${laneState(props.lane)})`} />
      <text class="route-line__label" x="500" y={props.labelY} text-anchor="middle">
        {label()}
        <Show when={probe()}>{(value) => <tspan x="500" dy="16">{probeLabel(props.lane)}: {value()}</tspan>}</Show>
      </text>
    </g>
  );
}

export function TopologyMap(props: TopologyMapProps) {
  const viewport = createViewport();
  const [vertical, setVertical] = createSignal(savedOrientation());
  createEffect(() => { const value = vertical(); try { localStorage.setItem("tunnel-orientation", value ? "vertical" : "horizontal"); } catch { /* Optional persistence. */ } });
  const foreign = () => props.cluster.nodes.filter((node) => node.node_class === "foreign");
  const domestic = () => props.cluster.nodes.filter((node) => node.node_class === "domestic");
  const maxRows = () => Math.max(foreign().length, domestic().length, 1);
  const mapHeight = () => maxRows() * 196 + 10;
  const laneNode = (id: string) => props.cluster.nodes.find((node) => node.id === id);
  const position = (node: ClusterNode) => nodeY((node.node_class === "foreign" ? foreign() : domestic()).findIndex((item) => item.id === node.id));
  const selectedLane = (id: string) => props.selectedKind === "lane" && props.selectedId === id;
  return (
    <section class="topology-panel" classList={{ "topology-panel--vertical": vertical() }} aria-label="Cluster visualizer">
      <div class="topology-viewport" ref={(element) => element.addEventListener("click", viewport.click, true)} onPointerDown={viewport.down} onPointerMove={viewport.move} onPointerUp={viewport.up} onPointerCancel={viewport.up}>
        <div class="topology-canvas" style={{ height: `${mapHeight()}px`, "--map-height": `${mapHeight()}px`, transform: `translate(${viewport.view().x}px, ${viewport.view().y}px) scale(${viewport.view().scale})` }}>
          <svg class="topology-routes" viewBox={`0 0 1000 ${mapHeight()}`} preserveAspectRatio="none" aria-label="Cluster connections">
            <defs>
              <marker id="arrow-healthy" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 Z" /></marker>
              <marker id="arrow-degraded" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 Z" /></marker>
              <marker id="arrow-unknown" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 Z" /></marker>
            </defs>
            <For each={props.cluster.lanes}>{(lane, index) => {
              const source = laneNode(lane.source_node_id);
              const peer = laneNode(lane.peer_node_id);
              return source && peer ? <Route lane={lane} reverse={source.node_class === "domestic"} sourceY={position(source)} peerY={position(peer)} labelY={(position(source) + position(peer)) / 2 - 12 + index() * 24} selected={selectedLane(lane.id)} onSelect={() => props.onLaneSelect(lane.id)} /> : null;
            }}</For>
          </svg>
          <div class="node-bank node-bank--foreign">
            <For each={foreign()}>{(node) => <NodeCard node={node} selected={props.selectedKind === "node" && props.selectedId === node.id} onSelect={() => props.onNodeSelect(node.id)} />}</For>
          </div>
          <div class="node-bank node-bank--domestic">
            <For each={domestic()}>{(node) => <NodeCard node={node} selected={props.selectedKind === "node" && props.selectedId === node.id} onSelect={() => props.onNodeSelect(node.id)} />}</For>
          </div>
        </div>
      </div>
      <div class="map-zoom" onClick={(event) => event.stopPropagation()}><button type="button" aria-label="Rotate graph layout" aria-pressed={vertical()} title="Rotate graph layout" onClick={() => { setVertical(!vertical()); viewport.reset(); }}><svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><rect x="7" y="4" width="10" height="16" rx="2" /><path d="M10 17h4M3 10a9 9 0 0 1 15-6m0-3v3h-3M21 14a9 9 0 0 1-15 6m0 3v-3h3" /></svg></button><button type="button" aria-label="Zoom out" onClick={() => viewport.zoom(1 / 1.2)}>−</button><span>{Math.round(viewport.view().scale * 100)}%</span><button type="button" aria-label="Zoom in" onClick={() => viewport.zoom(1.2)}>+</button></div>
    </section>
  );
}
