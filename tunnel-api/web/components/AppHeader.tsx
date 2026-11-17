import type { TelegramContext } from "../platform/telegram";

interface AppHeaderProps {
  context: TelegramContext;
  generatedAt: number | undefined;
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
      <div class="ops-header__identity">
        <span class="ops-header__mark" aria-hidden="true">۰۱</span>
        <div>
          <p class="eyebrow">TUNNEL / OBSERVATORY</p>
          <h1>مرکز مشاهدهٔ تونل</h1>
        </div>
      </div>
      <div class="ops-header__controls">
        <div>
          <strong>{source()}</strong>
          <span>{props.generatedAt ? "نقشه بر پایهٔ آخرین گزارش عامل" : "در انتظار گزارش عامل"}</span>
        </div>
        <button class="refresh-button" type="button" onClick={() => props.onRefresh()} disabled={props.loading || !props.refreshable}>
          {props.loading ? "در حال خواندن" : "بازخوانی"}
        </button>
      </div>
    </header>
  );
}
