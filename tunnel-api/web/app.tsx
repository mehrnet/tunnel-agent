import { For, Match, Show, Switch, createSignal } from "solid-js";
import { InspectorPanel } from "./components/InspectorPanel";
import { AddMenu } from "./components/AddMenu";
import { StatePanel } from "./components/StatePanel";
import { TopologyMap } from "./components/TopologyMap";
import { NewTunnel } from "./components/TunnelEditor";
import type { ClusterResponse, ClusterSummary, Selection } from "./data/types";
import { createClusterData } from "./lib/cluster-state";
import { telegramContext } from "./platform/telegram";

async function createCluster(name: string, initData?: string) {
  if (!initData) throw new Error("Cluster creation requires Telegram authentication.");
  const response = await fetch("/api/v1/web/clusters", { method: "POST", headers: { "content-type": "application/json", "x-telegram-init-data": initData }, body: JSON.stringify({ name }) });
  if (!response.ok) throw new Error("Could not create cluster.");
  return (await response.json() as { cluster: ClusterSummary }).cluster;
}

function Dashboard(props: { cluster: ClusterResponse; selection: Selection; onNodeSelect: (id: string) => void; onLaneSelect: (id: string) => void; initData: string; refresh: () => void }) {
  return <div class="observatory-grid">
    <TopologyMap cluster={props.cluster} selectedKind={props.selection?.kind} selectedId={props.selection?.id} onNodeSelect={props.onNodeSelect} onLaneSelect={props.onLaneSelect} />
    <Show when={props.selection}><div class="popup-shade"><div class="node-popup" role="dialog" aria-label="Details" onClick={(event) => event.stopPropagation()}><InspectorPanel cluster={props.cluster} selection={props.selection} initData={props.initData} refresh={props.refresh} /></div></div></Show>
  </div>;
}

export function App() {
  const context = telegramContext();
  const [selection, setSelection] = createSignal<Selection>();
  const { clusters, setClusters, selectedId, cluster, phase, loadError, chooseCluster, refresh } =
    createClusterData(context.initData, () => setSelection(undefined));
  const [adding, setAdding] = createSignal(false); const [creationError, setCreationError] = createSignal("");
  const [addingTunnel, setAddingTunnel] = createSignal(false);
  async function addCluster(event: SubmitEvent) {
    event.preventDefault();
    const name = String(new FormData(event.currentTarget as HTMLFormElement).get("name") ?? "").trim();
    try {
      const created = await createCluster(name, context.initData);
      setClusters([...clusters(), created]); setAdding(false); chooseCluster(created.id);
    } catch (error) { setCreationError(error instanceof Error ? error.message : "Could not create cluster."); }
  }

  return (
    <main class="app-shell" dir="ltr" data-theme={context.colorScheme} onClick={() => { setSelection(undefined); setAdding(false); }}>
      <nav class="cluster-tabs" aria-label="Clusters" onClick={(event) => event.stopPropagation()}>
        <For each={clusters()}>{(item) => <button type="button" classList={{ active: item.id === selectedId() }} onClick={() => chooseCluster(item.id)}>{item.name}{item.release_track === "testing" ? " · Testing" : ""}</button>}</For>
        <AddMenu clusterId={selectedId()} testing={clusters().some((item) => item.id === selectedId() && item.release_track === "testing")} initData={context.initData} refresh={refresh} newCluster={() => { setCreationError(""); setAdding(true); }}
          newTunnel={context.initData && (cluster()?.nodes.length ?? 0) >= 2 ? () => setAddingTunnel(true) : undefined} />
      </nav>
      <Show when={adding()}><div class="popup-shade"><form class="node-popup create-cluster" onClick={(event) => event.stopPropagation()} onSubmit={(event) => void addCluster(event)}><h2>New cluster</h2><input name="name" aria-label="Cluster name" placeholder="Cluster name" required maxlength={80} /><p>{creationError()}</p><button type="submit">Create cluster</button></form></div></Show>
      <Show when={addingTunnel() && cluster()}>{(snapshot) => <NewTunnel cluster={snapshot()} initData={context.initData ?? ""}
        close={() => setAddingTunnel(false)} saved={() => { setAddingTunnel(false); refresh(); }} />}</Show>
      <div class="app-workspace">
        <div class="app-workspace__content">
          <Switch>
            <Match when={phase() === "telegram"}><StatePanel state="telegram" /></Match>
            <Match when={phase() === "error"}><StatePanel state="error" detail={loadError()} onRetry={refresh} /></Match>
            <Match when={phase() === "empty"}><StatePanel state="empty" /></Match>
            <Match when={phase() === "loading" && !cluster()}><StatePanel state="loading" /></Match>
            <Match when={cluster()}>{(snapshot) => <Dashboard cluster={snapshot()} selection={selection()} initData={context.initData ?? ""} refresh={refresh} onNodeSelect={(id) => { queueMicrotask(() => setSelection({ kind: "node", id })); }} onLaneSelect={(id) => { queueMicrotask(() => setSelection({ kind: "lane", id })); }} />}</Match>
          </Switch>
        </div>
      </div>
    </main>
  );
}
