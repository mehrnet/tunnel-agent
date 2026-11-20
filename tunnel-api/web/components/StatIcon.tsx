const paths: Record<string, string> = {
  CPU: "M7 7h10v10H7z M9 1v4m6-4v4M9 19v4m6-4v4M1 9h4m-4 6h4m14-6h4m-4 6h4",
  MEM: "M3 6h18v12H3z M7 10v4m5-4v4m5-4v4M7 18v3m5-3v3m5-3v3",
  RX: "M12 3v14m-5-5 5 5 5-5M4 18v3h16v-3",
  TX: "M12 17V3m-5 5 5-5 5 5M4 18v3h16v-3",
  LOAD: "M2 12h5l3-8 4 16 3-8h5",
  RSS: "M4 4h16v16H4z M8 8h8v8H8z",
  Heartbeat: "M2 12h5l3-8 4 16 3-8h5",
  Address: "M3 12h18M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18M12 3c-5 5-5 13 0 18 5-5 5-13 0-18",
};

export function StatIcon(props: { label: string }) {
  return <svg class="stat-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={paths[props.label] ?? "M4 5h16v14H4z M8 9h8m-8 6h8"} /></svg>;
}
