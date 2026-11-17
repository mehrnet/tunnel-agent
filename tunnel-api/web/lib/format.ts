export function formatNumber(value: number, digits = 0): string {
  return new Intl.NumberFormat("fa-IR", { maximumFractionDigits: digits }).format(value);
}

export function formatPercent(value: number): string {
  return `${formatNumber(value)}٪`;
}

export function formatBytes(value: number): string {
  const units = ["بایت", "کیلوبایت", "مگابایت", "گیگابایت"];
  const index = Math.min(Math.floor(Math.log(Math.max(value, 1)) / Math.log(1024)), units.length - 1);
  return `${formatNumber(value / 1024 ** index, index === 0 ? 0 : 1)} ${units[index]}`;
}

export function formatRate(value: number): string {
  return `${formatBytes(value)}/ث`;
}

export function formatMoment(value: number | null): string {
  if (!value) return "گزارشی نرسیده";
  return new Intl.DateTimeFormat("fa-IR", { hour: "2-digit", minute: "2-digit" }).format(value);
}

export function formatProbe(value: number | null): string {
  return value === null ? "بی‌پاسخ" : `${formatNumber(value)} ms`;
}
