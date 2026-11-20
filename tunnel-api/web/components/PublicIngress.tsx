import { For, Show, createSignal } from "solid-js";
import { postOwner } from "../api/client";
import type { ClusterNode, ClusterResponse } from "../data/types";

export function PublicIngress(props: { node: ClusterNode; cluster: ClusterResponse; initData: string; refresh: () => void }) {
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal("");
  const lanes = () => props.cluster.lanes.filter((lane) =>
    lane.peer_node_id === props.node.id && lane.managed?.enabled && lane.managed.listener_type === "tcp");
  const routes = () => (props.cluster.public_ingress ?? []).filter((route) => route.node_id === props.node.id);
  const [port, setPort] = createSignal("");
  const route = () => routes().find((item) => String(item.port) === port());
  const choices = () => {
    const current = route();
    return current ? lanes().filter((lane) => [current.lane_id, current.fallback_lane_id].includes(lane.id)) : lanes();
  };

  async function update(event: SubmitEvent) {
    event.preventDefault();
    if (busy()) return;
    const fields = new FormData(event.currentTarget as HTMLFormElement);
    setBusy(true); setMessage("");
    try {
      await postOwner(`/clusters/${props.cluster.cluster.id}/public-ingress`, props.initData, {
        action: "set", node_id: props.node.id, port: Number(port()),
        revision: props.cluster.document_revisions?.[props.node.id],
        lane_id: fields.get("lane_id"), fallback_lane_id: fields.get("fallback_lane_id"),
        tls_server_name: fields.get("tls_server_name"),
      });
      setMessage("Requested. The agent switches only after a successful TLS check.");
      props.refresh();
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not update public port"); }
    finally { setBusy(false); }
  }

  return <Show when={lanes().length || routes().length}>
    <details class="node-lifecycle"><summary>Public TCP port routing</summary>
      <form onSubmit={(event) => void update(event)}>
        <label>Public port <input type="number" min="1" max="65535" required value={port()}
          onInput={(event) => setPort(event.currentTarget.value)} /></label>
        <Show when={routes().length}><p>Configured: <For each={routes()}>{(item) =>
          <button type="button" onClick={() => setPort(String(item.port))}>:{item.port}</button>}</For></p></Show>
        <label>Preferred lane <select name="lane_id" required value={route()?.lane_id ?? ""}>
          <option value="">Select active TCP lane</option>
          <For each={choices()}>{(lane) => <option value={lane.id}>{lane.transport} · {lane.local_listener}</option>}</For>
        </select></label>
        <Show when={!route()}><label>Fallback lane <select name="fallback_lane_id" required>
          <option value="">Select the existing lane</option>
          <For each={lanes()}>{(lane) => <option value={lane.id}>{lane.transport} · {lane.local_listener}</option>}</For>
        </select></label></Show>
        <label>Destination TLS name <input name="tls_server_name" required value={route()?.tls_server_name ?? ""} /></label>
        <p>Switch between the two reserved lanes. Existing TCP sessions are not rerouted.</p>
        <button type="submit" value="set" disabled={busy()}>Apply lane preference</button>
        <Show when={message()}><p role="status">{message()}</p></Show>
      </form>
    </details>
  </Show>;
}
