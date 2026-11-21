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
  const [laneId, setLaneId] = createSignal("");
  const route = () => routes().find((item) => String(item.port) === port());
  const primary = () => laneId() || route()?.lane_id || "";
  const fixed = () => Boolean(lanes().find((lane) => lane.id === primary())?.managed?.forward_udp || route()?.forward_udp);
  function selectPort(value: string) { setPort(value); setLaneId(""); }
  const choices = () => {
    const current = route();
    if (!current) return lanes();
    return lanes().filter((lane) => current.forward_udp ? !!lane.managed?.forward_udp
      : [current.lane_id, current.fallback_lane_id].includes(lane.id));
  };

  async function update(action: "set" | "remove", fields?: FormData) {
    if (busy()) return;
    setBusy(true); setMessage("");
    try {
      await postOwner(`/clusters/${props.cluster.cluster.id}/public-ingress`, props.initData, {
        action, node_id: props.node.id, port: Number(port()),
        revision: props.cluster.document_revisions?.[props.node.id],
        lane_id: fields?.get("lane_id"), fallback_lane_id: fields?.get("fallback_lane_id"),
        tls_server_name: fields?.get("tls_server_name"),
      });
      setMessage(action === "remove" || fixed() ? "Requested. Waiting for the agent to apply the change."
        : "Requested. The agent switches only after a successful TLS check.");
      props.refresh();
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not update public port"); }
    finally { setBusy(false); }
  }

  return <Show when={lanes().length || routes().length}>
    <details class="node-lifecycle"><summary>Public port routing</summary>
      <form onSubmit={(event) => { event.preventDefault(); void update("set", new FormData(event.currentTarget)); }}>
        <label>Public port <input type="number" min="1" max="65535" required value={port()}
          onInput={(event) => selectPort(event.currentTarget.value)} /></label>
        <Show when={routes().length}><p>Configured: <For each={routes()}>{(item) =>
          <button type="button" onClick={() => selectPort(String(item.port))}>:{item.port}</button>}</For></p></Show>
        <label>Preferred lane <select name="lane_id" required value={primary()} onChange={(event) => setLaneId(event.currentTarget.value)}>
          <option value="">Select active TCP lane</option>
          <For each={choices()}>{(lane) => <option value={lane.id}>{lane.transport} · {lane.local_listener}</option>}</For>
        </select></label>
        <Show when={!fixed() && !route()}><label>Fallback lane <select name="fallback_lane_id" required>
          <option value="">Select the existing lane</option>
          <For each={lanes().filter((lane) => !lane.managed?.forward_udp)}>{(lane) =>
            <option value={lane.id}>{lane.transport} · {lane.local_listener}</option>}</For>
        </select></label></Show>
        <Show when={!fixed()} fallback={<p>Fixed TCP+UDP forwarding. No customer TLS termination or automatic failover.</p>}>
          <label>Destination TLS name <input name="tls_server_name" required value={route()?.tls_server_name ?? ""} /></label>
          <p>Switch between the two reserved lanes. Existing TCP sessions are not rerouted.</p>
        </Show>
        <button type="submit" value="set" disabled={busy()}>Apply lane preference</button>
        <Show when={route()}><button type="button" disabled={busy()} onClick={() => void update("remove")}>Remove mapping</button></Show>
        <Show when={message()}><p role="status">{message()}</p></Show>
      </form>
    </details>
  </Show>;
}
