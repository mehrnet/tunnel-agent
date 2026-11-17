import { Match, Switch, createSignal, onMount } from "solid-js";
import { fetchCluster, fetchClusters } from "./api/client";
import { AppHeader } from "./components/AppHeader";
import { InspectorPanel } from "./components/InspectorPanel";
import { OperationsRail } from "./components/OperationsRail";
import { StatePanel } from "./components/StatePanel";
import { TopologyMap } from "./components/TopologyMap";
import { TriagePanel } from "./components/TriagePanel";
import { demoCluster, demoClusters } from "./data/demo";
import type { ClusterResponse, ClusterSummary, Selection } from "./data/types";
import { demoPreviewEnabled, telegramContext } from "./platform/telegram";

type DataPhase = "ready" | "loading" | "empty" | "error" | "telegram";

function initialSelection(cluster: ClusterResponse): Selection {
  const lane = cluster.lanes.find((item) => item.selected) ?? cluster.lanes[0];
  return lane ? { kind: "lane", id: lane.id } : cluster.nodes[0] ? { kind: "node", id: cluster.nodes[0].id } : undefined;
}

function Dashboard(props: { cluster: ClusterResponse; selection: Selection; onNodeSelect: (id: string) => void; onLaneSelect: (id: string) => void }) {
  return <div class="observatory-grid">
    <TopologyMap cluster={props.cluster} selectedKind={props.selection?.kind} selectedId={props.selection?.id} onNodeSelect={props.onNodeSelect} onLaneSelect={props.onLaneSelect} />
    <div class="observatory-grid__side"><InspectorPanel cluster={props.cluster} selection={props.selection} /><TriagePanel cluster={props.cluster} selection={props.selection} /></div>
  </div>;
}

export function App() {
  const context = telegramContext();
  const preview = context.source === "browser" && demoPreviewEnabled();
  const browserCluster = preview ? demoCluster(demoClusters[0].id) : undefined;
  const [clusters, setClusters] = createSignal<ClusterSummary[]>(preview ? demoClusters : []);
  const [selectedId, setSelectedId] = createSignal(browserCluster?.cluster.id ?? "");
  const [cluster, setCluster] = createSignal<ClusterResponse | undefined>(browserCluster);
  const [selection, setSelection] = createSignal<Selection>(browserCluster ? initialSelection(browserCluster) : undefined);
  const [phase, setPhase] = createSignal<DataPhase>(preview ? "ready" : context.source === "telegram" ? "loading" : "telegram");

  function applyCluster(next: ClusterResponse) {
    setCluster(next);
    setSelection(initialSelection(next));
    setPhase("ready");
  }

  async function loadTelemetry(id: string) {
    if (!context.initData) return;
    setPhase("loading");
    try {
      applyCluster(await fetchCluster(id, context.initData));
    } catch {
      setPhase("error");
    }
  }

  async function loadOwnedClusters() {
    if (!context.initData) return;
    setPhase("loading");
    try {
      const owned = await fetchClusters(context.initData);
      setClusters(owned);
      if (owned.length === 0) return setPhase("empty");
      const id = owned.find((item) => item.id === selectedId())?.id ?? owned[0].id;
      setSelectedId(id);
      await loadTelemetry(id);
    } catch {
      setPhase("error");
    }
  }

  function chooseCluster(id: string) {
    setSelectedId(id);
    if (preview) applyCluster(demoCluster(id));
    else void loadTelemetry(id);
  }

  function refresh() {
    if (preview) applyCluster(demoCluster(selectedId()));
    else if (context.source === "telegram") void loadTelemetry(selectedId());
  }

  onMount(() => { if (context.source === "telegram") void loadOwnedClusters(); });
  return (
    <main class="app-shell" dir="rtl" data-theme={context.colorScheme}>
      <OperationsRail cluster={cluster()} clusters={clusters()} selectedId={selectedId()} loading={phase() === "loading"} source={context.source} preview={preview} selectable={preview || context.source === "telegram"} onSelect={chooseCluster} />
      <div class="app-workspace">
        <AppHeader context={context} cluster={cluster()} loading={phase() === "loading"} preview={preview} refreshable={preview || context.source === "telegram"} onRefresh={refresh} />
        <div class="app-workspace__content">
          <Switch>
            <Match when={phase() === "telegram"}><StatePanel state="telegram" /></Match>
            <Match when={phase() === "error"}><StatePanel state="error" onRetry={refresh} /></Match>
            <Match when={phase() === "empty"}><StatePanel state="empty" /></Match>
            <Match when={phase() === "loading" && !cluster()}><StatePanel state="loading" /></Match>
            <Match when={cluster()}>{(snapshot) => <Dashboard cluster={snapshot()} selection={selection()} onNodeSelect={(id) => setSelection({ kind: "node", id })} onLaneSelect={(id) => setSelection({ kind: "lane", id })} />}</Match>
          </Switch>
        </div>
      </div>
    </main>
  );
}
