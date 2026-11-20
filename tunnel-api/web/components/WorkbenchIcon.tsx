const paths = {
  brand: "M5 5h5v5H5zM14 14h5v5h-5zM10 7.5h5l2 2V14M7.5 10v5l2 2H14",
  telegram: "m3 11 18-7-4 16-6-5-3 3v-6l9-5-7 7",
  server: "M4 4h16v7H4zM4 13h16v7H4zM7 7.5h1m-1 9h1m4-9h5m-5 9h5",
  lanes: "M4 7h15m-4-4 4 4-4 4m5 6H5m4-4-4 4 4 4",
  warning: "M12 3 2 20h20L12 3Zm0 6v5m0 3h.01",
  search: "M10.5 4a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13ZM16 16l4 4",
  chevron: "m9 5 7 7-7 7",
  close: "m6 6 12 12M18 6 6 18",
  rotate: "M7 4h10v16H7zM10 17h4M3 10a9 9 0 0 1 15-6m0-3v3h-3M21 14a9 9 0 0 1-15 6m0 3v-3h3",
  fit: "M9 4H4v5m11-5h5v5M4 15v5h5m11-5v5h-5",
  minus: "M5 12h14",
  plus: "M5 12h14m-7-7v14",
  map: "m3 6 6-3 6 3 6-3v15l-6 3-6-3-6 3V6Zm6-3v15m6-12v15",
  inventory: "M4 5h16M4 12h16M4 19h16M7 3v4m0 3v4m0 3v4",
  refresh: "M20 8a8 8 0 1 0 .5 7M20 3v5h-5",
} as const;

export function WorkbenchIcon(props: { name: keyof typeof paths }) {
  return <svg class="workbench-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={paths[props.name]} /></svg>;
}
