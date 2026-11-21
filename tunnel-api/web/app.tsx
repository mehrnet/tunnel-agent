import { For, Match, Show, Switch, createSignal } from "solid-js";
import { AddMenu } from "./components/AddMenu";
import { Dashboard } from "./components/Dashboard";
import { OwnerDialog } from "./components/OwnerDialog";
import { WorkbenchIcon } from "./components/WorkbenchIcon";
import { StatePanel } from "./components/StatePanel";
import { NewTunnel } from "./components/TunnelEditor";
import type { ClusterSummary, Selection } from "./data/types";
import { createClusterData } from "./lib/cluster-state";
import { telegramContext } from "./platform/telegram";

async function createCluster(name: string, initData?: string) {
  if (!initData) throw new Error("Cluster creation requires Telegram authentication.");
  const response = await fetch("/api/v1/web/clusters", { method: "POST", headers: { "content-type": "application/json", "x-telegram-init-data": initData }, body: JSON.stringify({ name }) });
  if (!response.ok) throw new Error("Could not create cluster.");
  return (await response.json() as { cluster: ClusterSummary }).cluster;
}

function Masthead(props: { telegram: boolean }) {
  return <header class="masthead">
    <div class="masthead__brand">
      <span class="masthead__mark"><WorkbenchIcon name="brand" /></span>
      <div><h1>Tunnel Owners</h1><p>Network operations</p></div>
    </div>
    <div class="masthead__context"><WorkbenchIcon name="telegram" />{props.telegram ? "Telegram session" : "Telegram access"}</div>
  </header>;
}

function ClusterDialog(props: { error: string; onClose: () => void; onSubmit: (event: SubmitEvent) => void }) {
  return <OwnerDialog class="node-popup details-dialog" title="create-title" close={props.onClose}>
    <form class="create-cluster" onSubmit={(event) => props.onSubmit(event)}>
      <h2 id="create-title">New cluster</h2>
      <label>Cluster name<input name="name" placeholder="Cluster name" required maxlength={80} /></label>
      <Show when={props.error}><p role="alert">{props.error}</p></Show>
      <div class="create-cluster__actions"><button type="button" onClick={() => props.onClose()}>Cancel</button><button type="submit">Create cluster</button></div>
    </form>
  </OwnerDialog>;
}

export function App() {
  const context = telegramContext();
  const [selection, setSelection] = createSignal<Selection>();
  const [inventory, setInventory] = createSignal(false);
  const { clusters, setClusters, selectedId, cluster, phase, loadError, chooseCluster, refresh, releaseOperator } =
    createClusterData(context.initData, () => { setSelection(undefined); setInventory(false); });
  const [adding, setAdding] = createSignal(false);
  const [creationError, setCreationError] = createSignal("");
  const [addingTunnel, setAddingTunnel] = createSignal(false);
  const snapshot = () => cluster()?.cluster.id === selectedId() ? cluster() : undefined;

  async function addCluster(event: SubmitEvent) {
    event.preventDefault();
    const name = String(new FormData(event.currentTarget as HTMLFormElement).get("name") ?? "").trim();
    try {
      const created = await createCluster(name, context.initData);
      setClusters([...clusters(), created]); setAdding(false); chooseCluster(created.id);
    } catch (error) { setCreationError(error instanceof Error ? error.message : "Could not create cluster."); }
  }

  return (
    <main class="app-shell" dir="ltr" data-theme={context.colorScheme}>
      <Show when={context.initData} fallback={<Masthead telegram={context.source === "telegram"} />}>
        <nav class="cluster-tabs" aria-label="Clusters">
          <h1 class="cluster-tabs__brand" title="Tunnel Owners"><WorkbenchIcon name="brand" /><span class="visually-hidden">Tunnel Owners</span></h1>
          <div class="cluster-tabs__list">
            <For each={clusters()} fallback={<span class="cluster-tabs__none">{phase() === "loading" ? "Loading clusters" : "No clusters"}</span>}>
              {(item) => <button type="button" aria-current={item.id === selectedId() ? "true" : undefined} onClick={() => chooseCluster(item.id)}>
                <span>{item.name}</span><Show when={item.release_track === "testing"}><small>Testing</small></Show>
              </button>}
            </For>
          </div>
          <Show when={snapshot() && phase() !== "error"}>
            <button class="workspace-toggle" type="button" aria-controls="cluster-workspace"
              onClick={() => setInventory(!inventory())}>
              <WorkbenchIcon name={inventory() ? "map" : "inventory"} />{inventory() ? "Map" : "Inventory"}
            </button>
          </Show>
          <AddMenu clusterId={selectedId()} testing={clusters().some((item) => item.id === selectedId() && item.release_track === "testing")}
            initData={context.initData} releaseOperator={releaseOperator()} refresh={refresh}
            newCluster={() => { setCreationError(""); setAdding(true); }}
            newTunnel={(cluster()?.nodes.length ?? 0) >= 2 ? () => setAddingTunnel(true) : undefined} />
        </nav>
      </Show>
      <Show when={adding()}><ClusterDialog error={creationError()} onClose={() => setAdding(false)} onSubmit={(event) => void addCluster(event)} /></Show>
      <Show when={addingTunnel() && cluster()}>{(current) => <NewTunnel cluster={current()} initData={context.initData ?? ""}
        close={() => setAddingTunnel(false)} saved={() => { setAddingTunnel(false); refresh(); }} />}</Show>
      <div id="cluster-workspace" class="app-workspace">
        <Switch>
          <Match when={phase() === "telegram"}><StatePanel state="telegram" /></Match>
          <Match when={phase() === "error"}><StatePanel state="error" detail={loadError()} onRetry={refresh} /></Match>
          <Match when={phase() === "empty"}><StatePanel state="empty" /></Match>
          <Match when={snapshot()}>{(current) => <Dashboard cluster={current()} inventory={inventory()} selection={selection()} initData={context.initData ?? ""}
            refresh={refresh} onSelect={setSelection} />}</Match>
          <Match when={phase() === "loading"}><StatePanel state="loading" /></Match>
        </Switch>
      </div>
    </main>
  );
}
