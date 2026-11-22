import { Index, Show, createMemo, createSignal, createUniqueId } from "solid-js";
import type { Selection } from "../data/types";
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

function routeStats(route: MappedLane) {
  const statuses = route.lanes.map(laneHealth);
  const degraded = statuses.filter((value) => value === "Degraded").length;
  const healthy = statuses.filter((value) => value === "Healthy").length;
  const status = degraded > 0 ? "Degraded" : healthy > 0 ? "Healthy" : statuses[0];
  return { status, degraded, healthy };
}

function RouteLabel(props: { route: MappedLane; reveal: boolean; selected: boolean; onSelect: () => void }) {
  const lane = () => props.route.lanes[0];
  const stats = createMemo(() => routeStats(props.route));
  const timing = () => stats().status === "Healthy" ? formatProbe(lane().probe_ms, lane().fresh_until) : null;
  const transport = () => props.route.bundleId ? `${props.route.lanes.length} ${routeKind(lane()).toLowerCase()} lanes`
    : [lane().transport, lane().direction?.replaceAll("_", " ")].filter(Boolean).join(" ");
  const readout = () => props.route.bundleId
    ? stats().degraded ? `${stats().degraded} degraded` : `${stats().healthy}/${props.route.lanes.length} healthy`
    : [listenerLabel(lane()), timing() ? `${timing()} RTT` : stats().status].filter(Boolean).join(" · ");
  const description = () => [
    transport(), `${props.route.source.name} to ${props.route.peer.name}`,
    !props.route.bundleId && laneStatus(lane()), readout(),
  ].filter(Boolean).join(", ");
  return <button type="button" class="route-label" data-kind={lane().kind ?? "tunnel"} data-status={stats().status}
    data-reveal={props.reveal} aria-label={description()} title={description()}
    aria-pressed={props.selected} style={{ left: `${props.route.x}px`, top: `${props.route.y}px` }} onClick={() => props.onSelect()}>
    <span aria-hidden="true" class="route-label__key" /><strong>{transport()}</strong><small>{readout()}</small>
  </button>;
}

export function MapRoutes(props: { routes: MappedLane[]; dense: boolean; selection: Selection; onSelect: (selection: Selection) => void }) {
  const arrow = `lane-arrow-${createUniqueId()}`;
  const routeId = (route: MappedLane) => route.bundleId ?? route.lanes[0].id;
  const [hovered, setHovered] = createSignal<string>();
  const [focused, setFocused] = createSignal<string>();
  const highlight = (route: MappedLane) => hovered() === routeId(route) || focused() === routeId(route);
  const selected = (route: MappedLane) => props.selection?.kind === "bundle" ? props.selection.id === route.bundleId
    : props.selection?.kind === "lane" && route.lanes.some((lane) => lane.id === props.selection?.id);
  const select = (route: MappedLane) => props.onSelect({ kind: route.bundleId ? "bundle" : "lane", id: routeId(route) });
  return <div class="map-routes" classList={{ "map-routes--dense": props.dense }} role="group" aria-label="Lanes">
    <svg class="topology-routes" aria-hidden="true">
      <defs><marker id={arrow} viewBox="0 0 8 8" markerWidth="7" markerHeight="7" refX="7" refY="4" orient="auto-start-reverse"><path d="M0 0 8 4 0 8Z" fill="context-stroke" /></marker></defs>
      <Index each={props.routes}>{(route) =>
        <g class="route-line" data-kind={route().lanes[0].kind ?? "tunnel"} data-status={routeStats(route()).status}
          data-selected={selected(route())} data-highlight={highlight(route())}>
          <path class="route-line__track" d={route().path} marker-end={`url(#${arrow})`} />
          <path class="route-line__hit" d={route().path} onClick={() => select(route())}
            onPointerEnter={() => setHovered(routeId(route()))} onPointerLeave={() => setHovered(undefined)} />
        </g>
      }</Index>
    </svg>
    <Index each={props.routes}>{(route) =>
      <div class="route-caption" onFocusIn={() => setFocused(routeId(route()))} onFocusOut={() => setFocused(undefined)}>
        <RouteLabel route={route()} reveal={highlight(route())} selected={selected(route())} onSelect={() => select(route())} />
      </div>
    }</Index>
  </div>;
}
