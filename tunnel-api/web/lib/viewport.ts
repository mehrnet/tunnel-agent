import { createEffect, createSignal } from "solid-js";

function savedView() {
  try {
    const value = JSON.parse(localStorage.getItem("tunnel-view") ?? "null");
    if (value && [value.x, value.y, value.scale].every(Number.isFinite) && value.scale >= 0.35 && value.scale <= 3) return { x: Number(value.x), y: Number(value.y), scale: Number(value.scale) };
  } catch { /* Storage may be unavailable in embedded browsers. */ }
  return { x: 0, y: 0, scale: 1 };
}

export function createViewport() {
  const [view, setView] = createSignal(savedView());
  createEffect(() => { const value = view(); try { localStorage.setItem("tunnel-view", JSON.stringify(value)); } catch { /* Optional persistence. */ } });
  const pointers = new Map<number, { x: number; y: number }>();
  let start = { x: 0, y: 0 };
  let moved = false;
  const point = (event: PointerEvent) => {
    const element = event.currentTarget as HTMLElement;
    const rect = element.getBoundingClientRect();
    return element.closest(".topology-panel--vertical")
      ? { x: event.clientY - rect.top, y: rect.right - event.clientX }
      : { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };
  const geometry = () => {
    const points = [...pointers.values()];
    const a = points[0]; const b = points[1] ?? a;
    return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2, distance: Math.hypot(a.x - b.x, a.y - b.y) };
  };
  function down(event: PointerEvent) {
    if (event.pointerType === "mouse" && event.button !== 0) return;
    if (!pointers.size) { moved = false; start = point(event); }
    pointers.set(event.pointerId, point(event));
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }
  function move(event: PointerEvent) {
    if (!pointers.has(event.pointerId)) return;
    const before = geometry();
    pointers.set(event.pointerId, point(event));
    const after = geometry();
    if (Math.hypot(after.x - start.x, after.y - start.y) < 4 && pointers.size === 1 && !moved) return;
    moved = true;
    const element = event.currentTarget as HTMLElement;
    element.setPointerCapture(event.pointerId);
    setView((old) => {
      const scale = Math.max(0.35, Math.min(3, old.scale * (before.distance > 0 ? after.distance / before.distance : 1)));
      const ratio = scale / old.scale;
      return { scale, x: after.x - (before.x - old.x) * ratio,
        y: after.y - (before.y - old.y) * ratio };
    });
  }
  function up(event: PointerEvent) { pointers.delete(event.pointerId); }
  function click(event: MouseEvent) {
    if (moved) { event.preventDefault(); event.stopPropagation(); }
  }
  const zoom = (factor: number) => setView((old) => ({ ...old, scale: Math.max(0.35, Math.min(3, old.scale * factor)) }));
  return { view, down, move, up, click, zoom, reset: () => setView({ x: 0, y: 0, scale: 1 }) };
}
