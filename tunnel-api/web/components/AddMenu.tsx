import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { postOwner } from "../api/client";
import { OwnerDialog } from "./OwnerDialog";

function ServerDialog(props: { busy: boolean; error: string; close: () => void; submit: (event: SubmitEvent) => void }) {
  return <OwnerDialog class="node-popup lane-editor" title="new-server-title" close={props.close}>
    <form onSubmit={(event) => props.submit(event)}>
      <header class="lane-editor__title"><div><p class="detail-kicker">ENROLLMENT</p><h2 id="new-server-title">New server</h2></div>
        <button type="button" onClick={() => props.close()}>Close</button></header>
      <label>Server name <input name="name" placeholder="e.g. de-2" required /></label>
      <label>IP address or hostname <input name="address" placeholder="e.g. 192.0.2.10" required /></label>
      <label>Server role <select name="role"><option value="domestic">Domestic (Iran)</option><option value="foreign">Foreign</option></select></label>
      <p>The server will appear as pending. Open its details to get the installation command.</p>
      <Show when={props.error}><p role="alert">{props.error}</p></Show>
      <button class="lane-editor__save" type="submit" disabled={props.busy}>{props.busy ? "Creating…" : "Create server"}</button>
    </form>
  </OwnerDialog>;
}

export function AddMenu(props: { clusterId: string; testing: boolean; initData?: string; newCluster: () => void; newTunnel?: () => void; refresh: () => void }) {
  const [open, setOpen] = createSignal(false);
  const [server, setServer] = createSignal(false);
  const [error, setError] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [confirmTrack, setConfirmTrack] = createSignal(false);
  let menu!: HTMLDivElement;
  let trigger!: HTMLButtonElement;
  function clearMenu() { setOpen(false); setConfirmTrack(false); setError(""); }
  onMount(() => {
    function outside(event: PointerEvent) {
      if (open() && event.target instanceof Node && !menu.contains(event.target)) clearMenu();
    }
    document.addEventListener("pointerdown", outside);
    onCleanup(() => document.removeEventListener("pointerdown", outside));
  });
  async function setTrack() {
    setBusy(true); setError("");
    try {
      await postOwner(`/clusters/${props.clusterId}/release-track`, props.initData ?? "", { track: props.testing ? "stable" : "testing" });
      clearMenu(); props.refresh();
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
  return <div class="add-menu" ref={menu} onKeyDown={(event) => {
    if (open() && event.key === "Escape") { clearMenu(); trigger.focus(); }
  }}>
    <button type="button" ref={trigger} aria-label="Add" aria-expanded={open()} onClick={() => { if (open()) clearMenu(); else setOpen(true); }}>+</button>
    <Show when={open()}><div class="add-menu__items">
      <button type="button" disabled={!props.clusterId} onClick={() => { clearMenu(); setServer(true); }}>New server</button>
      <button type="button" onClick={() => { clearMenu(); props.newCluster(); }}>New cluster</button>
      <Show when={props.newTunnel}><button type="button" onClick={() => { clearMenu(); props.newTunnel?.(); }}>New tunnel</button></Show>
      <Show when={props.clusterId && props.initData}>
        <Show when={confirmTrack()} fallback={<button type="button" onClick={() => setConfirmTrack(true)}>{props.testing ? "Use stable releases" : "Set as testing cluster"}</button>}>
          <p>Change the release track for every server in this cluster?</p>
          <button type="button" disabled={busy()} onClick={() => void setTrack()}>Confirm track change</button>
          <button type="button" disabled={busy()} onClick={() => setConfirmTrack(false)}>Cancel</button>
        </Show>
      </Show>
      <Show when={error()}><p role="alert">{error()}</p></Show>
    </div></Show>
    <Show when={server()}><ServerDialog busy={busy()} error={error()} close={() => setServer(false)} submit={(event) => void submit(event)} /></Show>
  </div>;
}
