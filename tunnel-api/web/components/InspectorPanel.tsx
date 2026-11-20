import { Show, createSignal } from "solid-js";
import { postOwner } from "../api/client";
import type { ClusterNode, ClusterResponse, Selection, TunnelLane } from "../data/types";
import { formatMoment, formatProbe, formatRate, laneState, laneStatus, probeLabel } from "../lib/format";
import { StatIcon } from "./StatIcon";
import { BootstrapField } from "./BootstrapField";
import { TunnelControls } from "./TunnelEditor";
import { PublicIngress } from "./PublicIngress";
import { ProbePanel } from "./ProbePanel";

interface InspectorPanelProps {
  cluster: ClusterResponse;
  selection: Selection;
  initData: string;
  refresh: () => void;
}

const enrollmentLabel = { enrolled: "Enrolled", pending: "Pending", revoked: "Revoked" };

function DetailRow(props: { label: string; value: string; mono?: boolean }) {
  return <div class="detail-row"><span><StatIcon label={props.label} />{props.label}</span><strong classList={{ mono: props.mono }}>{props.value}</strong></div>;
}

function LaneInspection(props: { lane: TunnelLane; nodes: ClusterNode[] }) {
  const source = () => props.nodes.find((node) => node.id === props.lane.source_node_id)?.name ?? "Unknown node";
  const peer = () => props.nodes.find((node) => node.id === props.lane.peer_node_id)?.name ?? "Unknown node";
  return <>
    <div class="inspection-status"><span class={`status-mark status-mark--${laneState(props.lane)}`}>{laneStatus(props.lane)}</span><code>{props.lane.id}</code></div>
    <div class="detail-list">
      <DetailRow label="Endpoints" value={`${source()} → ${peer()}`} />
      <DetailRow label="Transport" value={props.lane.transport} mono />
      <Show when={props.lane.priority !== null}><DetailRow label="Priority" value={String(props.lane.priority)} mono /></Show>
      <Show when={props.lane.local_listener}>{(listener) => <DetailRow label="Listener" value={listener()} mono />}</Show>
      <Show when={props.lane.probe_scope}>
        <DetailRow label={probeLabel(props.lane)} value={formatProbe(props.lane.probe_ms, props.lane.fresh_until) ?? "No recent timing"} mono />
      </Show>
      <DetailRow label="Last report" value={formatMoment(props.lane.observed_at)} />
    </div>
  </>;
}

function NodeLifecycle(props: { node: ClusterNode; initData: string; refresh: () => void }) {
  const [error, setError] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  async function apply(event: SubmitEvent) {
    event.preventDefault();
    if (busy()) return;
    const data = new FormData(event.currentTarget as HTMLFormElement);
    if (data.get("name") !== props.node.name) return setError("Type the exact server name to confirm.");
    setBusy(true); setError("");
    try {
      await postOwner(`/nodes/${props.node.id}/lifecycle`, props.initData, {
        action: data.get("action"), confirm_node_id: props.node.id,
      });
      props.refresh();
    } catch (reason) { setError(reason instanceof Error ? reason.message : "Server action failed"); }
    finally { setBusy(false); }
  }
  return <details class="node-lifecycle"><summary>Server recovery and retirement</summary>
    <form onSubmit={(event) => void apply(event)}>
      <p>These actions revoke the current enrollment identity. Remove managed lanes first before retiring a server.</p>
      <label>Action <select name="action">
        <option value="reinstall">Require reinstall</option><option value="rotate">Revoke credential and re-enroll</option>
        <option value="retire">Retire server</option>
      </select></label>
      <label>Confirm server name <input name="name" required autocomplete="off" /></label>
      <button type="submit" disabled={busy()}>Apply server action</button>
      <Show when={error()}><p role="alert">{error()}</p></Show>
    </form>
  </details>;
}

function NodeInspection(props: { node: ClusterNode; initData: string; refresh: () => void }) {
  const telemetry = () => props.node.telemetry;
  return <>
    <div class="inspection-status"><span class={`node-state node-state--${props.node.status}`}>{enrollmentLabel[props.node.status]}</span><code>{props.node.id}</code></div>
    <div class="detail-list">
      <DetailRow label="Role" value={props.node.node_class === "foreign" ? "Foreign node" : "Domestic node"} />
      <DetailRow label="Address" value={props.node.address ?? "Not set"} mono />
      <DetailRow label="Heartbeat" value={formatMoment(props.node.last_heartbeat_at)} />
      <Show when={telemetry()} fallback={<DetailRow label="Telemetry" value="No telemetry" />}>
        {(sample) => <DetailRow label="Host traffic" value={`${formatRate(sample().rx_bytes_per_sec)} RX / ${formatRate(sample().tx_bytes_per_sec)} TX`} />}
      </Show>
    </div>
    <Show when={props.initData && props.node.status !== "revoked"}><NodeLifecycle node={props.node} initData={props.initData} refresh={props.refresh} /></Show>
    <Show when={props.node.status === "pending"}>
      <BootstrapField nodeId={props.node.id} />
      <Show when={props.node.node_class === "domestic"}><BootstrapField nodeId={props.node.id} domesticGrant /></Show>
    </Show>
  </>;
}

export function InspectorPanel(props: InspectorPanelProps) {
  const selectedLane = () => props.selection?.kind === "lane" ? props.cluster.lanes.find((lane) => lane.id === props.selection?.id) : undefined;
  const selectedNode = () => props.selection?.kind === "node" ? props.cluster.nodes.find((node) => node.id === props.selection?.id) : undefined;
  return (
    <section class="inspection-panel" aria-labelledby="inspection-title">
      <div class="panel-heading"><div><p class="section-label">SELECTION</p><h2 id="inspection-title">Details</h2></div></div>
      <Show when={selectedLane()} fallback={<Show when={selectedNode()} fallback={<p class="inspection-empty">Select a node or connection.</p>}>{(node) => <>
        <NodeInspection node={node()} initData={props.initData} refresh={props.refresh} />
        <Show when={props.initData && node().status === "enrolled"}><PublicIngress node={node()} cluster={props.cluster} initData={props.initData} refresh={props.refresh} /></Show>
      </>}</Show>}>
        {(lane) => <><LaneInspection lane={lane()} nodes={props.cluster.nodes} />
          <TunnelControls lane={lane()} clusterId={props.cluster.cluster.id} initData={props.initData} saved={props.refresh} />
          <ProbePanel lane={lane()} cluster={props.cluster} initData={props.initData} refresh={props.refresh} /></>}
      </Show>
    </section>
  );
}
