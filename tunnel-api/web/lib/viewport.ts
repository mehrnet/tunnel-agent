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
  let multitouch = false;
  let touchInput = false;
  let tapTarget: Element | null = null;
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
    if (!pointers.size) {
      moved = false;
      multitouch = false;
      start = point(event);
      touchInput = event.pointerType === "touch";
      tapTarget = touchInput && event.target instanceof Element
        ? event.target.closest("button, .route-line__hit") : null;
    }
    pointers.set(event.pointerId, point(event));
    multitouch ||= pointers.size > 1;
    if (touchInput && event.target instanceof Element) event.target.setPointerCapture(event.pointerId);
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
  function up(event: PointerEvent) {
    if (!pointers.delete(event.pointerId) || pointers.size) return;
    const target = tapTarget;
    tapTarget = null;
    if (event.type !== "pointerup" || !target?.isConnected || moved || multitouch) return;
    // Some WebViews suppress the next compatibility click after a drag.
    target.dispatchEvent(new MouseEvent("click", { bubbles: true, composed: true }));
  }
  function leave(event: PointerEvent) {
    const host = event.currentTarget as HTMLElement;
    if (event.pointerType === "mouse" && !host.hasPointerCapture(event.pointerId)) up(event);
  }
  function click(event: MouseEvent) {
    if (event.detail !== 0 && (moved || multitouch || touchInput)) {
      event.preventDefault();
      event.stopPropagation();
    }
  }
  const zoom = (factor: number) => setView((old) => ({ ...old, scale: Math.max(0.35, Math.min(3, old.scale * factor)) }));
  return { view, down, move, up, leave, click, zoom, reset: () => setView({ x: 0, y: 0, scale: 1 }) };
}
