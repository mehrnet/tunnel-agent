import { Show } from "solid-js";
import type { ClusterNode, ClusterResponse, Selection, TunnelLane } from "../data/types";
import { formatBytes, formatNumber, formatPercent, formatProbe } from "../lib/format";

interface TriagePanelProps {
  cluster: ClusterResponse;
  selection: Selection;
}

const conditionText = { healthy: "پروب پاسخ‌گو است", degraded: "پروب کند یا ناپایدار است", unknown: "برای وضعیت مسیر گزارشی نیست" };

function selectedItems(cluster: ClusterResponse, selection: Selection) {
  const lane = selection?.kind === "lane" ? cluster.lanes.find((item) => item.id === selection.id) : undefined;
  const node = selection?.kind === "node" ? cluster.nodes.find((item) => item.id === selection.id) : undefined;
  const host = node ?? cluster.nodes.find((item) => item.id === lane?.source_node_id);
  return { host, lane };
}

function HostPressure(props: { node: ClusterNode | undefined }) {
  const telemetry = () => props.node?.telemetry;
  return <article class="triage-column triage-column--host">
    <p class="section-label">فشار میزبان</p>
    <h3>{props.node ? props.node.name : "گره انتخاب نشده"}</h3>
    <Show when={telemetry()} fallback={<p>برای این عامل تله‌متری میزبان نرسیده است؛ نبود داده، سلامت یا خرابی مسیر را اثبات نمی‌کند.</p>}>
      {(sample) => <>
        <div class="pressure-readout"><b>CPU {formatPercent(sample().cpu_percent)}</b><b>MEM {formatPercent(sample().memory_percent)}</b><b>بار {formatNumber(sample().load_1, 2)}</b></div>
        <p>RX/TX و RSS عامل در اینجا مصرف کل میزبان‌اند: {formatBytes(sample().agent_rss_bytes)} حافظهٔ عامل ثبت شده است.</p>
      </>}
    </Show>
  </article>;
}

function LaneCondition(props: { lane: TunnelLane | undefined }) {
  return <article class="triage-column triage-column--lane">
    <p class="section-label">شرایط لین</p>
    <h3>{props.lane ? props.lane.transport : "لین انتخاب نشده"}</h3>
    <Show when={props.lane} fallback={<p>با انتخاب یک خط، تأخیر پروب و زمان گزارش همان مسیر در اینجا خوانده می‌شود.</p>}>
      {(lane) => <>
        <div class={`condition-readout condition-readout--${lane().state}`}><b>{formatProbe(lane().probe_ms)}</b><span>{conditionText[lane().state]}</span></div>
        <p>این سنجه فقط به همین لین مربوط است و با بار سایر پردازش‌های میزبان یکی نیست.</p>
      </>}
    </Show>
  </article>;
}

export function TriagePanel(props: TriagePanelProps) {
  const items = () => selectedItems(props.cluster, props.selection);
  return <section class="triage-panel" aria-labelledby="triage-title">
    <div class="panel-heading"><div><p class="section-label">راهنمای تریاژ</p><h2 id="triage-title">اول میزبان را از لین جدا کنید</h2></div></div>
    <p class="triage-panel__intro">فشار بالای میزبان می‌تواند از بار کاری دیگری بیاید؛ کاهش کیفیت لین را فقط از پروب همان مسیر نتیجه بگیرید.</p>
    <div class="triage-grid"><HostPressure node={items().host} /><LaneCondition lane={items().lane} /></div>
  </section>;
}
