import { Index, createMemo } from "solid-js";
import type { ClusterResponse } from "../data/types";
import { laneBundleId } from "../lib/topology-layout";
import { laneHealth, listenerLabel, routeKind } from "../lib/workbench-status";
import { WorkbenchIcon } from "./WorkbenchIcon";

export function LaneBundlePanel(props: {
  cluster: ClusterResponse;
  id: string;
  onSelect: (id: string) => void;
  onClose: () => void;
}) {
  const lanes = createMemo(() => props.cluster.lanes.filter((lane) => laneBundleId(lane) === props.id));
  const first = () => lanes()[0];
  const nodeName = (id: string | undefined) => props.cluster.nodes.find((node) => node.id === id)?.name ?? "Unknown server";
  return <section class="owner-details" aria-labelledby="inspection-title">
    <header class="owner-details__title">
      <div><p class="detail-kicker">{lanes().length} {first() ? routeKind(first()).toUpperCase() : ""} LANES</p>
        <h2 id="inspection-title">{first() ? `${nodeName(first().source_node_id)} \u2192 ${nodeName(first().peer_node_id)}` : "Lane bundle"}</h2></div>
      <button type="button" class="owner-details__close" aria-label="Close details" onClick={() => props.onClose()}><WorkbenchIcon name="close" /></button>
    </header>
    <ul class="bundle-list">
      <Index each={lanes()} fallback={<li class="inspection-empty">No lanes in this bundle.</li>}>{(lane) =>
        <li><button type="button" class="bundle-list__row" onClick={() => props.onSelect(lane().id)}>
          <span class="inventory-row__text"><strong>{[lane().transport, listenerLabel(lane())].filter(Boolean).join(" / ")}</strong>
            <small>{lane().managed?.destination_host ? `${lane().managed?.destination_host}:${lane().managed?.destination_port}` : lane().direction?.replaceAll("_", " ")}</small></span>
          <span class="inventory-row__status" data-status={laneHealth(lane())}>{laneHealth(lane())}</span>
          <WorkbenchIcon name="chevron" />
        </button></li>
      }</Index>
    </ul>
  </section>;
}
