import { Index, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { ClusterResponse, Selection } from "../data/types";
import { createViewport } from "../lib/viewport";
import { topologyLayout } from "../lib/topology-layout";
import { MapNode, MapRoutes } from "./MapFeatures";
import { WorkbenchIcon } from "./WorkbenchIcon";

function useOrientation() {
  const media = window.matchMedia("(max-width: 900px)");
  const [narrow, setNarrow] = createSignal(media.matches);
  const [rotated, setRotated] = createSignal(false);
  try {
    setRotated(localStorage.getItem("tunnel-orientation") === "vertical");
  } catch { /* Storage is optional. */ }
  const resize = () => setNarrow(media.matches);
  onMount(() => {
    media.addEventListener("change", resize);
    onCleanup(() => media.removeEventListener("change", resize));
  });
  return {
    portrait: () => narrow() && !rotated(),
    rotated,
    rotate: () => {
      const next = setRotated((current) => !current);
      try { localStorage.setItem("tunnel-orientation", next ? "vertical" : "horizontal"); } catch { /* Storage is optional. */ }
    },
  };
}

function useFitMode() {
  const [fitted, setFitted] = createSignal(false);
  const key = "tunnel-fit";
  try { setFitted(localStorage.getItem(key) === "true"); } catch { /* Storage is optional. */ }
  return {
    fitted,
    toggleFit: () => {
      const next = setFitted((current) => !current);
      try { localStorage.setItem(key, String(next)); } catch { /* Storage is optional. */ }
    },
  };
}

function useMapCamera() {
  const viewport = createViewport();
  const [size, setSize] = createSignal({ width: 0, height: 0 });
  const attach = (element: HTMLDivElement) => {
    const observer = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      if (width && height) setSize({ width, height });
    });
    element.addEventListener("click", viewport.click, true);
    observer.observe(element);
    onCleanup(() => { element.removeEventListener("click", viewport.click, true); observer.disconnect(); });
  };
  return { ...viewport, size, attach };
}

function MapLegend() {
  return <ul class="map-legend" aria-label="Lane legend">
    <li data-line="customer">Customer</li><li data-line="management">Management</li>
    <li data-line="degraded">Degraded</li><li data-line="unknown">No current probe</li>
  </ul>;
}

function MapControls(props: {
  rotated: boolean;
  fitted: boolean;
  scale: number;
  renderedScale: number;
  onRotate: () => void;
  onFit: () => void;
  onZoom: (factor: number) => void;
}) {
  return <div class="map-controls" role="group" aria-label="Map view controls">
    <button type="button" aria-label="Rotate map layout" aria-pressed={props.rotated} title="Rotate layout" onClick={() => props.onRotate()}><WorkbenchIcon name="rotate" /></button>
    <button type="button" aria-label="Fit all servers and lanes" aria-pressed={props.fitted} title={props.fitted ? "Restore readable view" : "Fit all servers and lanes"} onClick={() => props.onFit()}><WorkbenchIcon name="fit" /></button>
    <button type="button" aria-label="Zoom out" title="Zoom out" onClick={() => props.onZoom(1 / 1.2)} disabled={props.scale <= 0.35}><WorkbenchIcon name="minus" /></button>
    <output aria-label="Map zoom">{Math.round(props.renderedScale * 100)}%</output>
    <button type="button" aria-label="Zoom in" title="Zoom in" onClick={() => props.onZoom(1.2)} disabled={props.scale >= 3}><WorkbenchIcon name="plus" /></button>
  </div>;
}

export function TopologyMap(props: {
  cluster: ClusterResponse;
  selection: Selection;
  onSelect: (selection: Selection) => void;
}) {
  const camera = useMapCamera();
  const { portrait, rotated, rotate } = useOrientation();
  const layout = createMemo(() => topologyLayout(props.cluster, portrait(), rotated()));
  const [focusOffset, setFocusOffset] = createSignal({ x: 0, y: 0 });
  const { fitted, toggleFit } = useFitMode();
  const isFitted = () => fitted() && camera.view().scale === 1 && !camera.view().x && !camera.view().y && !focusOffset().x && !focusOffset().y;
  const reset = () => { setFocusOffset({ x: 0, y: 0 }); camera.reset(); };
  const placement = createMemo(() => {
    const { width, height } = camera.size();
    const fit = width && height ? Math.min(1, width / layout().width, height / layout().height) : 1;
    const scale = fitted() ? fit : Math.max(rotated() ? 0.58 : 0.78, fit);
    const x = Math.max(0, (width - scale * layout().width) / 2);
    const y = Math.max(0, Math.min(44, (height - scale * layout().height) / 2));
    const view = camera.view();
    return {
      scale: scale * view.scale,
      x: view.x + view.scale * x + focusOffset().x,
      y: view.y + view.scale * y + focusOffset().y,
    };
  });
  const followFocus = (event: FocusEvent) => {
    if (!(event.target instanceof HTMLElement) || !event.target.matches(":focus-visible")) return;
    const item = event.target.getBoundingClientRect();
    const frame = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const reveal = (start: number, end: number, low: number, high: number) => start < low + 8 ? low + 8 - start : Math.min(0, high - 8 - end);
    const x = reveal(item.left, item.right, frame.left, frame.right);
    const y = reveal(item.top, item.bottom, frame.top, frame.bottom);
    setFocusOffset((old) => ({ x: old.x + (rotated() ? y : x), y: old.y + (rotated() ? -x : y) }));
  };
  const selectRoad = (id: string) => props.onSelect({ kind: "lane", id });
  const selectNode = (id: string) => props.onSelect({ kind: "node", id });
  return <div class="topology-frame"><section id="topology-panel" class="topology-panel" classList={{ "topology-panel--vertical": rotated() }}
    aria-labelledby="topology-title" data-layout={portrait() ? "portrait" : "landscape"}>
    <header class="workspace-panel-heading">
      <div><p class="section-label">Topology</p><h2 id="topology-title">Network map</h2></div>
      <MapLegend />
    </header>
    <div class="topology-viewport" ref={camera.attach} onPointerDown={camera.down} onPointerMove={camera.move}
      onPointerUp={camera.up} onPointerCancel={camera.up} onPointerLeave={camera.leave} onFocusIn={followFocus}>
      <div class="topology-grid" style={{ "--grid-step": `${24 * placement().scale}px`, "--grid-x": `${placement().x}px`, "--grid-y": `${placement().y}px` }} />
      <Show when={props.cluster.nodes.length} fallback={
        <div class="map-empty"><WorkbenchIcon name="map" /><strong>No servers to map</strong><p>Add a server from the cluster menu to start building the topology.</p></div>
      }>
        <div class="topology-canvas" style={{ width: `${layout().width}px`, height: `${layout().height}px`,
          transform: `translate(${placement().x}px, ${placement().y}px) scale(${placement().scale})` }}>
          <MapRoutes routes={layout().routes} selectedId={props.selection?.kind === "lane" ? props.selection.id : undefined} onSelect={selectRoad} />
          <div class="node-bank" role="group" aria-label="Domestic servers"
            style={{ "--bank-x": `${layout().domesticLabel.x}px`, "--bank-y": `${layout().domesticLabel.y}px` }}>
            <h3>Domestic <small>{layout().domestic.length}</small></h3>
            <Index each={layout().domestic} fallback={<p>No domestic servers</p>}>{(item) =>
              <MapNode item={item()} selected={props.selection?.kind === "node" && props.selection.id === item().node.id} onSelect={selectNode} />
            }</Index>
          </div>
          <div class="node-bank" role="group" aria-label="Foreign servers"
            style={{ "--bank-x": `${layout().foreignLabel.x}px`, "--bank-y": `${layout().foreignLabel.y}px` }}>
            <h3>Foreign <small>{layout().foreign.length}</small></h3>
            <Index each={layout().foreign} fallback={<p>No foreign servers</p>}>{(item) =>
              <MapNode item={item()} selected={props.selection?.kind === "node" && props.selection.id === item().node.id} onSelect={selectNode} />
            }</Index>
          </div>
        </div>
      </Show>
    </div>
    <footer class="map-footer">
      <span class="map-footer__hint">{layout().unmapped ? `${layout().unmapped} unmapped in inventory` : "Drag to pan"}<span> / </span><span>{portrait() ? "Pinch to zoom" : "Select to inspect"}</span></span>
      <MapControls rotated={rotated()} fitted={isFitted()} scale={camera.view().scale} renderedScale={placement().scale}
        onRotate={() => { rotate(); reset(); }} onFit={() => { if (!fitted() || isFitted()) toggleFit(); reset(); }} onZoom={camera.zoom} />
    </footer>
  </section></div>;
}
