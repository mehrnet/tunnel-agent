import { Show } from "solid-js";
import type { ClusterNode, ClusterResponse, Selection, TunnelLane } from "../data/types";
import { formatMoment, formatProbe, formatRate } from "../lib/format";

interface InspectorPanelProps {
  cluster: ClusterResponse;
  selection: Selection;
}

const stateLabel = { healthy: "پایدار", degraded: "کاهش کیفیت", unknown: "نامشخص" };
const enrollmentLabel = { enrolled: "ثبت‌شده", pending: "در انتظار", revoked: "لغوشده" };

function DetailRow(props: { label: string; value: string; mono?: boolean }) {
  return <div class="detail-row"><span>{props.label}</span><strong classList={{ mono: props.mono }}>{props.value}</strong></div>;
}

function LaneInspection(props: { lane: TunnelLane; nodes: ClusterNode[] }) {
  const source = () => props.nodes.find((node) => node.id === props.lane.source_node_id)?.name ?? "گره نامشخص";
  const peer = () => props.nodes.find((node) => node.id === props.lane.peer_node_id)?.name ?? "گره نامشخص";
  return <>
    <div class="inspection-status"><span class={`status-mark status-mark--${props.lane.state}`}>{stateLabel[props.lane.state]}</span><code>{props.lane.id}</code></div>
    <div class="detail-list">
      <DetailRow label="دو سر مسیر" value={`${source()} ← ${peer()}`} />
      <DetailRow label="ترابری / اولویت" value={`${props.lane.transport} / ${props.lane.priority}`} mono />
      <DetailRow label="شنوندهٔ محلی" value={props.lane.local_listener} mono />
      <DetailRow label="پروب پاسخ" value={formatProbe(props.lane.probe_ms)} mono />
      <DetailRow label="آخرین گزارش" value={formatMoment(props.lane.observed_at)} />
    </div>
  </>;
}

function NodeInspection(props: { node: ClusterNode }) {
  const telemetry = () => props.node.telemetry;
  return <>
    <div class="inspection-status"><span class={`node-state node-state--${props.node.status}`}>{enrollmentLabel[props.node.status]}</span><code>{props.node.id}</code></div>
    <div class="detail-list">
      <DetailRow label="نقش" value={props.node.node_class === "foreign" ? "گره خارجی" : "گره داخلی"} />
      <DetailRow label="نشانی" value={props.node.address ?? "ثبت نشده"} mono />
      <DetailRow label="آخرین ضربان" value={formatMoment(props.node.last_heartbeat_at)} />
      <Show when={telemetry()} fallback={<DetailRow label="تله‌متری" value="عامل هنوز ارتقا نیافته است" />}>
        {(sample) => <DetailRow label="ترافیک میزبان" value={`${formatRate(sample().rx_bytes_per_sec)} دریافت / ${formatRate(sample().tx_bytes_per_sec)} ارسال`} />}
      </Show>
    </div>
  </>;
}

export function InspectorPanel(props: InspectorPanelProps) {
  const selectedLane = () => props.selection?.kind === "lane" ? props.cluster.lanes.find((lane) => lane.id === props.selection?.id) : undefined;
  const selectedNode = () => props.selection?.kind === "node" ? props.cluster.nodes.find((node) => node.id === props.selection?.id) : undefined;
  return (
    <section class="inspection-panel" aria-labelledby="inspection-title">
      <div class="panel-heading"><div><p class="section-label">انتخاب فعلی</p><h2 id="inspection-title">بازرسی دقیق</h2></div></div>
      <Show when={selectedLane()} fallback={<Show when={selectedNode()} fallback={<p class="inspection-empty">یک مسیر یا گره را از نقشه انتخاب کنید.</p>}>{(node) => <NodeInspection node={node()} />}</Show>}>
        {(lane) => <LaneInspection lane={lane()} nodes={props.cluster.nodes} />}
      </Show>
    </section>
  );
}
