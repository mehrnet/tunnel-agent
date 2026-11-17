import type { ClusterResponse } from "../data/types";
import { formatMoment } from "../lib/format";
import type { TelegramContext } from "../platform/telegram";

interface AppHeaderProps {
  context: TelegramContext;
  cluster: ClusterResponse | undefined;
  loading: boolean;
  preview: boolean;
  refreshable: boolean;
  onRefresh: () => void;
}

export function AppHeader(props: AppHeaderProps) {
  const source = () => {
    if (props.context.source === "telegram") return "دادهٔ زندهٔ تلگرام";
    return props.preview ? "پیش‌نمایش محلی (?demo=1)" : "ربات تلگرام لازم است";
  };
  return (
    <header class="ops-header">
      <div class="ops-header__title">
        <div>
          <p class="eyebrow">TUNNEL / OBSERVATORY / ROUTE MAP</p>
          <h1>{props.cluster?.cluster.name ?? "مرکز مشاهدهٔ تونل"}</h1>
          <p class="ops-header__breadcrumb">عملیات خوشه <span aria-hidden="true">/</span> تله‌متری مسیر</p>
        </div>
      </div>
      <div class="ops-header__controls">
        <div class="ops-header__readout">
          <span class="ops-header__signal" classList={{ "ops-header__signal--live": props.context.source === "telegram" || props.preview }}><i aria-hidden="true" />{source()}</span>
          <span>{props.cluster ? "نقشه بر پایهٔ آخرین گزارش عامل" : "در انتظار گزارش عامل"}</span>
        </div>
        <div class="ops-header__generated"><span>برداشت</span><strong dir="ltr">{props.cluster ? formatMoment(props.cluster.generated_at) : "--:--"}</strong></div>
        <button class="refresh-button" type="button" onClick={() => props.onRefresh()} disabled={props.loading || !props.refreshable}>
          {props.loading ? "در حال خواندن" : "بازخوانی"}
        </button>
      </div>
    </header>
  );
}
