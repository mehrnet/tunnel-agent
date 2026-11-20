import { Show, createSignal } from "solid-js";
import { postOwner } from "../api/client";

export function AddMenu(props: { clusterId: string; testing: boolean; initData?: string; newCluster: () => void; newTunnel?: () => void; refresh: () => void }) {
  const [open, setOpen] = createSignal(false);
  const [server, setServer] = createSignal(false);
  const [error, setError] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  async function setTrack() {
    setBusy(true); setError("");
    try {
      await postOwner(`/clusters/${props.clusterId}/release-track`, props.initData ?? "", { track: props.testing ? "stable" : "testing" });
      setOpen(false); props.refresh();
    } catch (caught) { setError(caught instanceof Error ? caught.message : "Could not change release track"); }
    finally { setBusy(false); }
  }
  async function submit(event: SubmitEvent) {
    event.preventDefault(); if (busy()) return; setBusy(true); setError("");
    const values = Object.fromEntries(new FormData(event.currentTarget as HTMLFormElement));
    try {
      const response = await fetch("/api/v1/web/nodes", { method: "POST", headers: { "content-type": "application/json", "x-telegram-init-data": props.initData ?? "" }, body: JSON.stringify({ ...values, cluster_id: props.clusterId }) });
      if (!response.ok) throw new Error(`Could not create server (HTTP ${response.status}).`);
      setServer(false); props.refresh();
    } catch (caught) { setError(caught instanceof Error ? caught.message : "Request failed."); }
    finally { setBusy(false); }
  }
  return <div class="add-menu" onClick={(event) => event.stopPropagation()}>
    <button type="button" aria-label="Add" aria-expanded={open()} onClick={() => setOpen(!open())}>+</button>
    <Show when={open()}><div class="add-menu__items"><button type="button" disabled={!props.clusterId} onClick={() => { setOpen(false); setServer(true); }}>New Server</button><button type="button" onClick={() => { setOpen(false); props.newCluster(); }}>New Cluster</button><Show when={props.newTunnel}><button type="button" onClick={() => { setOpen(false); props.newTunnel?.(); }}>New Tunnel</button></Show>
      <Show when={props.clusterId && props.initData}><button type="button" disabled={busy()} onClick={() => void setTrack()}>{props.testing ? "Use stable releases" : "Set as testing cluster"}</button></Show>
      <Show when={error()}><p role="alert">{error()}</p></Show>
    </div></Show>
    <Show when={server()}><div class="popup-shade" onClick={() => setServer(false)}><form class="node-popup create-cluster" onClick={(event) => event.stopPropagation()} onSubmit={(event) => void submit(event)}><h2>New Server</h2><input name="name" placeholder="Server name" aria-label="Server name" required /><input name="address" placeholder="IP address or hostname" aria-label="Server address" required /><select name="role" aria-label="Server role"><option value="domestic">Domestic (Iran)</option><option value="foreign">Foreign</option></select><p>{error()}</p><button type="submit" disabled={busy()}>{busy() ? "Creating…" : "Create server"}</button></form></div></Show>
  </div>;
}
