import { For } from "solid-js";
import type { ClusterSummary } from "../data/types";

interface ClusterSelectorProps {
  clusters: ClusterSummary[];
  selectedId: string;
  loading: boolean;
  source: "telegram" | "browser";
  onSelect: (id: string) => void;
}

export function ClusterSelector(props: ClusterSelectorProps) {
  const caption = () => props.source === "telegram" ? "خوشه‌های مالک" : "دادهٔ آزمایشی محلی";
  return (
    <section class="cluster-bar" aria-label="انتخاب خوشه">
      <div class="cluster-bar__label">
        <span class="section-label">دامنهٔ مشاهده</span>
        <strong>{caption()}</strong>
      </div>
      <label class="cluster-select">
        <span>خوشه</span>
        <select value={props.selectedId} disabled={props.loading || props.clusters.length === 0} onChange={(event) => props.onSelect(event.currentTarget.value)}>
          <For each={props.clusters}>{(cluster) => <option value={cluster.id}>{cluster.name}</option>}</For>
        </select>
      </label>
    </section>
  );
}
