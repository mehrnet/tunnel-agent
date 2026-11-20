import { For, Show, createSignal } from "solid-js";
import { postOwner } from "../api/client";
import type { ClusterNode, ClusterResponse, Selection, TunnelLane } from "../data/types";
import { formatBytes, formatMoment, formatNumber, formatPercent, formatProbe, formatRate, laneState, laneStatus, probeLabel } from "../lib/format";
import { nodeStatus } from "../lib/workbench-status";
import { StatIcon } from "./StatIcon";
import { BootstrapField } from "./BootstrapField";
import { TunnelControls } from "./TunnelEditor";
import { PublicIngress } from "./PublicIngress";
import { ProbePanel } from "./ProbePanel";
import "../styles/details.css";

interface InspectorPanelProps {
  cluster: ClusterResponse;
  selection: Selection;
  initData: string;
  refresh: () => void;
  onClose?: () => void;
}

const enrollmentLabel = { enrolled: "Enrolled", pending: "Pending", revoked: "Revoked" };

function DetailRow(props: { label: string; value: string; mono?: boolean }) {
  return <div class="detail-row"><span><StatIcon label={props.label} />{props.label}</span><strong classList={{ mono: props.mono }}>{props.value}</strong></div>;
}

function serverStatus(node: ClusterNode) {
  const status = nodeStatus(node);
  if (status === "Active") return { title: status, tone: "healthy" };
  if (status === "Stalled" || status === "Revoked") return { title: status, tone: "degraded" };
  return { title: status === "Pending" ? "Waiting for install" : status, tone: "unknown" };
}

function LaneInspection(props: { lane: TunnelLane; nodes: ClusterNode[] }) {
  const source = () => props.nodes.find((node) => node.id === props.lane.source_node_id)?.name ?? "Unknown node";
  const peer = () => props.nodes.find((node) => node.id === props.lane.peer_node_id)?.name ?? "Unknown node";
  return <>
    <div class="inspection-status"><span class={`detail-status detail-status--${laneState(props.lane)}`}>{laneStatus(props.lane)}</span></div>
    <div class="detail-list">
      <DetailRow label="Connection" value={`${source()} → ${peer()}`} />
      <Show when={props.lane.priority !== null}><DetailRow label="Priority" value={String(props.lane.priority)} mono /></Show>
      <Show when={props.lane.local_listener}>{(listener) => <DetailRow label="Listener" value={listener()} mono />}</Show>
      <Show when={props.lane.managed?.destination_host}>{(host) =>
        <DetailRow label="Destination" value={`${host()}:${props.lane.managed?.destination_port}`} mono />}
      </Show>
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

function HostReadout(props: { node: ClusterNode }) {
  const metrics = () => {
    const sample = props.node.telemetry;
    if (!sample) return [];
    return [
      ["CPU", formatPercent(sample.cpu_percent)], ["MEM", formatPercent(sample.memory_percent)],
      ["RX", formatRate(sample.rx_bytes_per_sec)], ["TX", formatRate(sample.tx_bytes_per_sec)],
      ["LOAD", formatNumber(sample.load_1, 2)], ["RSS", formatBytes(sample.agent_rss_bytes)],
    ];
  };
  return <section class="host-readout" aria-label="Host telemetry">
    <header><h3>Host telemetry</h3><span>{formatMoment(props.node.telemetry?.sampled_at ?? null)}</span></header>
    <div class="host-readout__grid"><For each={metrics()} fallback={<p>No host telemetry has been reported.</p>}>
      {([label, value]) => <div><span><StatIcon label={label} />{label}</span><strong>{value}</strong></div>}
    </For></div>
  </section>;
}

function NodeInspection(props: { node: ClusterNode }) {
  const state = () => serverStatus(props.node);
  return <>
    <div class="inspection-status"><span class={`detail-status detail-status--${state().tone}`}>{state().title}</span></div>
    <div class="detail-list">
      <DetailRow label="Address" value={props.node.address ?? "Not set"} mono />
      <DetailRow label="Heartbeat" value={formatMoment(props.node.last_heartbeat_at)} />
      <DetailRow label="Enrollment" value={enrollmentLabel[props.node.status]} />
    </div>
    <HostReadout node={props.node} />
  </>;
}

export function InspectorPanel(props: InspectorPanelProps) {
  const selectedLane = () => props.selection?.kind === "lane" ? props.cluster.lanes.find((lane) => lane.id === props.selection?.id) : undefined;
  const selectedNode = () => props.selection?.kind === "node" ? props.cluster.nodes.find((node) => node.id === props.selection?.id) : undefined;
  const title = () => selectedNode()?.name ?? [selectedLane()?.transport, selectedLane()?.direction].filter(Boolean).join(" ");
  const label = () => selectedLane() ? selectedLane()?.kind === "management" ? "MANAGEMENT LANE" : "CUSTOMER LANE"
    : selectedNode()?.node_class === "foreign" ? "FOREIGN SERVER" : "DOMESTIC SERVER";
  return (
    <section class="inspection-panel owner-details" aria-labelledby="inspection-title">
      <header class="owner-details__title">
        <div><p class="detail-kicker">{label()}</p><h2 id="inspection-title">{title() || "Details"}</h2></div>
        <Show when={props.onClose}><button type="button" aria-label="Close details" class="owner-details__close" onClick={() => props.onClose?.()}>
          <svg width="18" height="18" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="m5 5 10 10M15 5 5 15" /></svg>
        </button></Show>
      </header>
      <Show when={selectedLane()} fallback={<Show when={selectedNode()} fallback={<p class="inspection-empty">Select a node or connection.</p>}>{(node) => <>
        <NodeInspection node={node()} />
        <Show when={props.initData && node().status === "enrolled"}><PublicIngress node={node()} cluster={props.cluster} initData={props.initData} refresh={props.refresh} /></Show>
        <Show when={props.initData && node().status !== "revoked"}><NodeLifecycle node={node()} initData={props.initData} refresh={props.refresh} /></Show>
        <Show when={node().status === "pending"}>
          <BootstrapField nodeId={node().id} initData={props.initData} />
          <Show when={node().node_class === "domestic"}><BootstrapField nodeId={node().id} initData={props.initData} domesticGrant /></Show>
        </Show>
      </>}</Show>}>
        {(lane) => <><LaneInspection lane={lane()} nodes={props.cluster.nodes} />
          <ProbePanel lane={lane()} cluster={props.cluster} initData={props.initData} refresh={props.refresh} />
          <Show when={lane().managed && props.initData}><details class="node-lifecycle"><summary>Manage lane</summary>
            <TunnelControls lane={lane()} clusterId={props.cluster.cluster.id} initData={props.initData} saved={props.refresh} />
          </details></Show></>}
      </Show>
    </section>
  );
}
