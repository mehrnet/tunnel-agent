import { For, Show } from "solid-js";
import type { ClusterNode, ClusterResponse, TunnelLane } from "../data/types";
import { formatBytes, formatMoment, formatNumber, formatPercent, formatProbe, formatRate } from "../lib/format";

interface TopologyMapProps {
  cluster: ClusterResponse;
  selectedKind: "lane" | "node" | undefined;
  selectedId: string | undefined;
  onNodeSelect: (id: string) => void;
  onLaneSelect: (id: string) => void;
}

const nodeStatus = { enrolled: "ثبت‌شده", pending: "در انتظار", revoked: "لغوشده" };
const laneStatus = { healthy: "پایدار", degraded: "کاهش کیفیت", unknown: "نامشخص" };

function laneClass(lane: TunnelLane, selected: boolean) {
  return `route-line route-line--${lane.state}${selected ? " route-line--selected" : ""}`;
}

function nodeY(index: number) {
  return 72 + index * 154;
}

function NodeCard(props: { node: ClusterNode; selected: boolean; onSelect: () => void }) {
  const telemetry = () => props.node.telemetry;
  const classLabel = () => props.node.node_class === "foreign" ? "بانک خارجی" : "بانک داخلی";
  return (
    <button class="map-node" classList={{ "map-node--selected": props.selected }} type="button" aria-pressed={props.selected} onClick={() => props.onSelect()}>
      <span class="map-node__topline"><span>{classLabel()}</span><b>{nodeStatus[props.node.status]}</b></span>
      <strong>{props.node.name}</strong>
      <code>{props.node.address ?? "نشانی ثبت نشده"}</code>
      <span class="map-node__heartbeat">ضربان: {formatMoment(props.node.last_heartbeat_at)}</span>
      <Show when={telemetry()} fallback={<span class="map-node__missing">عامل هنوز تله‌متری نمی‌فرستد</span>}>
        {(sample) => <div class="map-node__metrics">
          <span>CPU <b>{formatPercent(sample().cpu_percent)}</b></span>
          <span>MEM <b>{formatPercent(sample().memory_percent)}</b></span>
          <span>RX <b>{formatRate(sample().rx_bytes_per_sec)}</b></span>
          <span>TX <b>{formatRate(sample().tx_bytes_per_sec)}</b></span>
          <span>LOAD <b>{formatNumber(sample().load_1, 2)}</b></span>
          <span>RSS <b>{formatBytes(sample().agent_rss_bytes)}</b></span>
        </div>}
      </Show>
    </button>
  );
}

function Route(props: { lane: TunnelLane; sourceY: number; peerY: number; labelY: number; selected: boolean; onSelect: () => void }) {
  const keySelect = (event: KeyboardEvent) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      props.onSelect();
    }
  };
  return (
    <g class={laneClass(props.lane, props.selected)} role="button" tabindex="0" aria-pressed={props.selected} aria-label={`مسیر ${props.lane.transport}، ${laneStatus[props.lane.state]}`} onClick={() => props.onSelect()} onKeyDown={keySelect}>
      <line class="route-line__hit" x1="232" y1={props.sourceY} x2="768" y2={props.peerY} />
      <line class="route-line__track" x1="232" y1={props.sourceY} x2="768" y2={props.peerY} marker-end={`url(#arrow-${props.lane.state})`} />
      <text class="route-line__label" x="500" y={props.labelY} text-anchor="middle">{props.lane.transport} · {laneStatus[props.lane.state]}</text>
    </g>
  );
}

function RouteRail(props: { lane: TunnelLane; source: ClusterNode | undefined; peer: ClusterNode | undefined; selected: boolean; onSelect: () => void }) {
  const sourceName = () => props.source?.name ?? "گره نامشخص";
  const peerName = () => props.peer?.name ?? "گره نامشخص";
  return <button class={`route-rail route-rail--${props.lane.state}`} classList={{ "route-rail--selected": props.selected }} type="button" aria-pressed={props.selected} onClick={() => props.onSelect()}>
    <span class="route-rail__endpoints" dir="ltr"><b>{sourceName()}</b><i aria-hidden="true">→</i><b>{peerName()}</b></span>
    <span class="route-rail__meta"><small>{props.lane.transport} · {laneStatus[props.lane.state]}</small><strong>{formatProbe(props.lane.probe_ms)}</strong></span>
  </button>;
}

export function TopologyMap(props: TopologyMapProps) {
  const foreign = () => props.cluster.nodes.filter((node) => node.node_class === "foreign");
  const domestic = () => props.cluster.nodes.filter((node) => node.node_class === "domestic");
  const maxRows = () => Math.max(foreign().length, domestic().length, 1);
  const mapHeight = () => maxRows() * 154 + 10;
  const laneNode = (id: string) => props.cluster.nodes.find((node) => node.id === id);
  const position = (node: ClusterNode) => nodeY((node.node_class === "foreign" ? foreign() : domestic()).findIndex((item) => item.id === node.id));
  const selectedLane = (id: string) => props.selectedKind === "lane" && props.selectedId === id;
  return (
    <section class="topology-panel" aria-labelledby="topology-title">
      <div class="panel-heading">
        <div><p class="section-label">نقشهٔ جریان</p><h2 id="topology-title">توپولوژی و وضعیت مسیرها</h2></div>
        <div class="topology-summary"><span><b dir="ltr">{foreign().length}</b> خارج</span><span><b dir="ltr">{domestic().length}</b> داخل</span><span class="direction-note">خارج <i aria-hidden="true">←</i> داخل</span></div>
      </div>
      <div class="topology-viewport">
        <div class="topology-canvas" style={{ height: `${mapHeight()}px` }}>
          <svg class="topology-routes" viewBox={`0 0 1000 ${mapHeight()}`} preserveAspectRatio="none" aria-label="مسیرهای میان گره‌های خارجی و داخلی">
            <defs>
              <marker id="arrow-healthy" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 Z" /></marker>
              <marker id="arrow-degraded" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 Z" /></marker>
              <marker id="arrow-unknown" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 Z" /></marker>
            </defs>
            <For each={props.cluster.lanes}>{(lane, index) => {
              const source = laneNode(lane.source_node_id);
              const peer = laneNode(lane.peer_node_id);
              return source && peer ? <Route lane={lane} sourceY={position(source)} peerY={position(peer)} labelY={25 + index() * 18} selected={selectedLane(lane.id)} onSelect={() => props.onLaneSelect(lane.id)} /> : null;
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
      <div class="route-rail-list" aria-label="مسیرهای متصل به دو سر خود">
        <For each={props.cluster.lanes}>{(lane) => <RouteRail lane={lane} source={laneNode(lane.source_node_id)} peer={laneNode(lane.peer_node_id)} selected={selectedLane(lane.id)} onSelect={() => props.onLaneSelect(lane.id)} />}</For>
      </div>
      <div class="route-key" aria-label="راهنمای وضعیت مسیر">
        <span><i class="route-key__line route-key__line--healthy" />پایدار</span>
        <span><i class="route-key__line route-key__line--degraded" />کاهش کیفیت</span>
        <span><i class="route-key__line route-key__line--unknown" />نامشخص</span>
        <small>برای بازرسی، روی مسیر یا گره بزنید.</small>
      </div>
    </section>
  );
}
