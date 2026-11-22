import { Show } from "solid-js";
import type { ClusterResponse, Selection } from "../data/types";
import { InspectorPanel } from "./InspectorPanel";
import { InventoryPanel } from "./InventoryPanel";
import { LaneBundlePanel } from "./LaneBundlePanel";
import { OwnerDialog } from "./OwnerDialog";
import { TopologyMap } from "./TopologyMap";

export function Dashboard(props: {
  cluster: ClusterResponse;
  inventory: boolean;
  selection: Selection;
  initData: string;
  refresh: () => void;
  onSelect: (selection: Selection) => void;
}) {
  const close = () => props.onSelect(undefined);
  return <div class="workbench" data-view={props.inventory ? "inventory" : "map"}>
    <TopologyMap cluster={props.cluster} selection={props.selection} onSelect={props.onSelect} />
    <Show when={props.inventory}>
      <InventoryPanel cluster={props.cluster} selection={props.selection} onSelect={props.onSelect} />
    </Show>
    <Show when={props.selection}>
      <OwnerDialog class="node-popup details-dialog" title="inspection-title" close={close}>
        <Show when={props.selection?.kind === "bundle" && props.selection.id}
          fallback={<InspectorPanel cluster={props.cluster} selection={props.selection} initData={props.initData} refresh={props.refresh} onClose={close} />}>
          {(id) => <LaneBundlePanel cluster={props.cluster} id={id()} onSelect={(laneId) => props.onSelect({ kind: "lane", id: laneId })} onClose={close} />}
        </Show>
      </OwnerDialog>
    </Show>
  </div>;
}
