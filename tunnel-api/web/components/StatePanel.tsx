interface StatePanelProps {
  state: "loading" | "empty" | "error" | "telegram";
  onRetry?: () => void;
}

const messages = {
  loading: ["در حال خواندن نقشهٔ خوشه", "دادهٔ عامل‌ها و مسیرها در حال دریافت است."],
  empty: ["خوشه‌ای برای مشاهده وجود ندارد", "این حساب هنوز هیچ خوشهٔ قابل مشاهده‌ای ندارد."],
  error: ["دادهٔ خوشه در دسترس نیست", "اتصال تلگرام یا گزارش عامل را بررسی کنید، سپس دوباره بخوانید."],
  telegram: ["این مشاهده‌گر را در ربات Tunnel Owners Telegram باز کنید", "برای دیدن دادهٔ زندهٔ خوشه، این صفحه را از دکمهٔ ربات Tunnel Owners در تلگرام باز کنید."],
};

export function StatePanel(props: StatePanelProps) {
  const message = () => messages[props.state];
  return (
    <section class={`state-panel state-panel--${props.state}`} aria-live="polite">
      <span class="state-panel__mark" aria-hidden="true">{props.state === "error" ? "!" : props.state === "telegram" ? "TG" : "·"}</span>
      <div>
        <h2>{message()[0]}</h2>
        <p>{message()[1]}</p>
      </div>
      {props.state === "error" && props.onRetry ? <button type="button" onClick={props.onRetry}>تلاش دوباره</button> : null}
    </section>
  );
}
