import { Show, createSignal } from "solid-js";
import type { ClusterResponse, Selection } from "../data/types";
import { InspectorPanel } from "./InspectorPanel";
import { InventoryPanel } from "./InventoryPanel";
import { OwnerDialog } from "./OwnerDialog";
import { TopologyMap } from "./TopologyMap";
import { WorkbenchIcon } from "./WorkbenchIcon";
import { laneHealth, nodeStatus } from "../lib/workbench-status";
import { formatMoment } from "../lib/format";

function Overview(props: { snapshot: ClusterResponse }) {
  const active = () => props.snapshot.nodes.filter((node) => nodeStatus(node) === "Active").length;
  const healthy = () => props.snapshot.lanes.filter((lane) => laneHealth(lane) === "Healthy").length;
  const degraded = () => props.snapshot.lanes.filter((lane) => laneHealth(lane) === "Degraded").length;
  return <section class="cluster-overview" aria-labelledby="overview-title">
    <div class="cluster-overview__identity">
      <p class="section-label">Cluster overview</p>
      <h2 id="overview-title">{props.snapshot.cluster.name}</h2>
      <p class="cluster-overview__snapshot">Snapshot {formatMoment(props.snapshot.generated_at)}</p>
    </div>
    <dl class="cluster-overview__stats">
      <div><dt><WorkbenchIcon name="server" /> Servers</dt><dd><strong>{props.snapshot.nodes.length}</strong><span>{active()} active</span></dd></div>
      <div><dt><WorkbenchIcon name="lanes" /> Lanes</dt><dd><strong>{props.snapshot.lanes.length}</strong><span>{healthy()} healthy</span></dd></div>
      <div class="cluster-overview__degraded" classList={{ "has-degraded": degraded() > 0 }}>
        <dt><WorkbenchIcon name="warning" /> Degraded</dt><dd><strong>{degraded()}</strong><span>{degraded() === 1 ? "lane" : "lanes"}</span></dd>
      </div>
    </dl>
  </section>;
}

export function Dashboard(props: {
  cluster: ClusterResponse;
  selection: Selection;
  initData: string;
  refresh: () => void;
  onSelect: (selection: Selection) => void;
}) {
  const [tab, setTab] = createSignal<"map" | "inventory">("map");
  const close = () => props.onSelect(undefined);
  return <div class="workbench">
    <Overview snapshot={props.cluster} />
    <div class="workbench-switch" role="group" aria-label="Workspace view">
      <button type="button" aria-controls="topology-panel" aria-pressed={tab() === "map"} onClick={() => setTab("map")}><WorkbenchIcon name="map" />Map</button>
      <button type="button" aria-controls="inventory-panel" aria-pressed={tab() === "inventory"} onClick={() => setTab("inventory")}><WorkbenchIcon name="inventory" />Inventory</button>
    </div>
    <div class="workbench-grid" data-view={tab()}>
      <TopologyMap cluster={props.cluster} selection={props.selection} onSelect={props.onSelect} />
      <InventoryPanel cluster={props.cluster} selection={props.selection} onSelect={props.onSelect} />
    </div>
    <Show when={props.selection}>
      <OwnerDialog class="node-popup details-dialog" title="inspection-title" close={close}>
        <InspectorPanel cluster={props.cluster} selection={props.selection} initData={props.initData} refresh={props.refresh} onClose={close} />
      </OwnerDialog>
    </Show>
  </div>;
}
