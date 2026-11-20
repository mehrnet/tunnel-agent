import { Index, Show, createUniqueId } from "solid-js";
import { formatBytes, formatMoment, formatNumber, formatPercent, formatProbe, formatRate, laneStatus } from "../lib/format";
import type { MappedLane, MappedNode } from "../lib/topology-layout";
import { laneHealth, listenerLabel, nodeStatus, routeKind } from "../lib/workbench-status";
import { StatIcon } from "./StatIcon";
import { WorkbenchIcon } from "./WorkbenchIcon";

function Metric(props: { label: string; value: string }) {
  return <span><em><StatIcon label={props.label} />{props.label}</em><b>{props.value}</b></span>;
}

export function MapNode(props: { item: MappedNode; selected: boolean; onSelect: (id: string) => void }) {
  const node = () => props.item.node;
  const status = () => nodeStatus(node());
  return <button class="map-node" type="button" aria-pressed={props.selected}
    style={{ left: `${props.item.x}px`, top: `${props.item.y}px` }} onClick={() => props.onSelect(node().id)}>
    <span class="map-node__topline"><WorkbenchIcon name="server" /><b data-status={status()}>{status()}</b></span>
    <strong title={node().name}>{node().name}</strong>
    <code title={node().address ?? undefined}>{node().address ?? "No address"}</code>
    <span class="map-node__heartbeat">Heartbeat: {formatMoment(node().last_heartbeat_at)}</span>
    <Show when={node().telemetry} fallback={<span class="map-node__missing">No host telemetry</span>}>
      {(sample) => <span class="map-node__metrics">
        <Metric label="CPU" value={formatPercent(sample().cpu_percent)} />
        <Metric label="MEM" value={formatPercent(sample().memory_percent)} />
        <Metric label="RX" value={formatRate(sample().rx_bytes_per_sec)} />
        <Metric label="TX" value={formatRate(sample().tx_bytes_per_sec)} />
        <Metric label="LOAD" value={formatNumber(sample().load_1, 2)} />
        <Metric label="RSS" value={formatBytes(sample().agent_rss_bytes)} />
      </span>}
    </Show>
  </button>;
}

function RouteLabel(props: { route: MappedLane; selected: boolean; onSelect: (id: string) => void }) {
  const lane = () => props.route.lane;
  const status = () => laneHealth(lane());
  const timing = () => status() === "Healthy" ? formatProbe(lane().probe_ms, lane().fresh_until) : null;
  const transport = () => [lane().transport, lane().direction?.replaceAll("_", " ")].filter(Boolean).join(" ");
  const readout = () => [listenerLabel(lane()), timing() ? `${timing()} RTT` : status()].filter(Boolean).join(" · ");
  const description = () => [
    `${routeKind(lane())} lane`, lane().transport, `${props.route.source.name} to ${props.route.peer.name}`,
    laneStatus(lane()), lane().direction?.replaceAll("_", " "), readout(),
  ].filter(Boolean).join(", ");
  return <button type="button" class="route-label" data-kind={lane().kind ?? "tunnel"} data-status={status()}
    aria-label={description()} title={`${description()}${lane().local_listener ? ` / ${lane().local_listener}` : ""}`}
    aria-pressed={props.selected} style={{ left: `${props.route.x}px`, top: `${props.route.y}px` }} onClick={() => props.onSelect(lane().id)}>
    <span aria-hidden="true" class="route-label__key" /><strong>{transport()}</strong><small>{readout()}</small>
  </button>;
}

export function MapRoutes(props: { routes: MappedLane[]; selectedId?: string; onSelect: (id: string) => void }) {
  const arrow = `lane-arrow-${createUniqueId()}`;
  return <div class="map-routes" role="group" aria-label="Lanes">
    <svg class="topology-routes" aria-hidden="true">
      <defs><marker id={arrow} viewBox="0 0 8 8" markerWidth="7" markerHeight="7" refX="7" refY="4" orient="auto-start-reverse"><path d="M0 0 8 4 0 8Z" fill="context-stroke" /></marker></defs>
      <Index each={props.routes}>{(route) =>
        <g class="route-line" data-kind={route().lane.kind ?? "tunnel"} data-status={laneHealth(route().lane)} data-selected={props.selectedId === route().lane.id}>
          <path class="route-line__track" d={route().path} marker-end={`url(#${arrow})`} />
          <path class="route-line__hit" d={route().path} onClick={() => props.onSelect(route().lane.id)} />
        </g>
      }</Index>
    </svg>
    <Index each={props.routes}>{(route) =>
      <RouteLabel route={route()} selected={props.selectedId === route().lane.id} onSelect={props.onSelect} />
    }</Index>
  </div>;
}
