import { Show } from "solid-js";
import type { ClusterResponse, ClusterSummary } from "../data/types";
import { ClusterSelector } from "./ClusterSelector";

interface OperationsRailProps {
  cluster: ClusterResponse | undefined;
  clusters: ClusterSummary[];
  selectedId: string;
  loading: boolean;
  source: "telegram" | "browser";
  preview: boolean;
  selectable: boolean;
  onSelect: (id: string) => void;
}

export function OperationsRail(props: OperationsRailProps) {
  const sourceText = () => {
    if (props.source === "telegram") return "پیوند زنده برقرار است";
    return props.preview ? "نمایش دادهٔ نمونه" : "دسترسی تلگرام لازم است";
  };
  const stats = () => {
    const snapshot = props.cluster;
    return {
      nodes: snapshot?.nodes.length,
      lanes: snapshot?.lanes.length,
      watch: snapshot?.lanes.filter((lane) => lane.state !== "healthy").length,
    };
  };
  const metric = (value: number | undefined) => value === undefined ? "--" : value.toString();
  return (
    <aside class="ops-rail" aria-label="ناوبری عملیات خوشه">
      <header class="rail-brand">
        <span class="rail-brand__mark" aria-hidden="true"><i /><i /><i /><i /></span>
        <div><strong>TUNNEL OWNERS</strong><span>OPS CONSOLE</span></div>
      </header>
      <section class="rail-connection" aria-label="وضعیت اتصال">
        <span class="rail-connection__dot" classList={{ "rail-connection__dot--live": props.source === "telegram" || props.preview }} />
        <div><span>وضعیت نشست</span><strong>{sourceText()}</strong></div>
      </section>
      <Show when={props.selectable}>
        <ClusterSelector clusters={props.clusters} selectedId={props.selectedId} loading={props.loading} source={props.source} onSelect={props.onSelect} />
      </Show>
      <section class="rail-summary" aria-label="خلاصهٔ خوشه">
        <p class="section-label">نمای عملیاتی</p>
        <div>
          <span><strong dir="ltr">{metric(stats().nodes)}</strong><small>گره</small></span>
          <span><strong dir="ltr">{metric(stats().lanes)}</strong><small>لین</small></span>
          <span class="rail-summary__watch"><strong dir="ltr">{metric(stats().watch)}</strong><small>نیاز بررسی</small></span>
        </div>
      </section>
      <section class="rail-legend" aria-label="راهنمای وضعیت">
        <p class="section-label">راهنمای سیگنال</p>
        <span><i class="rail-legend__dot rail-legend__dot--healthy" />پایدار</span>
        <span><i class="rail-legend__dot rail-legend__dot--degraded" />کاهش کیفیت</span>
        <span><i class="rail-legend__dot rail-legend__dot--unknown" />بی‌گزارش</span>
      </section>
      <p class="rail-source"><b>منبع</b> عامل‌های ثبت‌شده و پروب مستقل لین</p>
    </aside>
  );
}
