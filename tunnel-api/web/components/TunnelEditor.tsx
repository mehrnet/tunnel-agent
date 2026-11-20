import { For, Show, createSignal } from "solid-js";
import { postOwner } from "../api/client";
import type { ClusterResponse, TunnelLane } from "../data/types";

function TcpDestination(props: { host?: string | null; port?: number | null }) {
  return <>
    <label>Destination from connector <input name="destination_host" required value={props.host ?? "127.0.0.1"} /></label>
    <label>Destination port <input name="destination_port" required type="number" min="1" max="65535" value={props.port ?? ""} /></label>
  </>;
}

export function NewTunnel(props: { cluster: ClusterResponse; initData: string; close: () => void; saved: () => void }) {
  const [kind, setKind] = createSignal("tcp");
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal("");
  const nodes = () => props.cluster.nodes.filter((node) => node.status !== "revoked");
  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (busy()) return;
    const fields = new FormData(event.currentTarget as HTMLFormElement);
    setBusy(true); setMessage("");
    try {
      await postOwner(`/clusters/${props.cluster.cluster.id}/tunnels`, props.initData, {
        source_node_id: fields.get("source"), listener_node_id: fields.get("listener"), transport: "wstunnel",
        listener_type: kind(), listener_port: Number(fields.get("listener_port")), control_port: Number(fields.get("control_port")),
        destination_host: fields.get("destination_host"), destination_port: Number(fields.get("destination_port")),
      });
      props.saved();
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not create lane"); }
    finally { setBusy(false); }
  }
  return <div class="popup-shade" onClick={() => props.close()}>
    <form class="node-popup create-cluster" onClick={(event) => event.stopPropagation()} onSubmit={(event) => void save(event)}>
      <h2>New reverse tunnel</h2>
      <label>Connector <select name="source" required><option value="">Select server</option><For each={nodes()}>{(node) => <option value={node.id}>{node.name}</option>}</For></select></label>
      <label>Listening server <select name="listener" required><option value="">Select server</option><For each={nodes()}>{(node) => <option value={node.id}>{node.name}</option>}</For></select></label>
      <label>Transport <select aria-label="Transport"><option>wstunnel (TLS)</option></select></label>
      <label>Loopback listener type <select value={kind()} onChange={(event) => setKind(event.currentTarget.value)}><option value="tcp">TCP</option><option value="socks5">SOCKS5</option></select></label>
      <label>Loopback port <input name="listener_port" type="number" min="1024" max="65535" required /></label>
      <label>TLS control port <input name="control_port" type="number" min="1024" max="65535" required /></label>
      <Show when={kind() === "tcp"}><TcpDestination /></Show>
      <p>Saved disabled. Before enabling, install the listener's TLS certificate and key and restrict the control port to the connector.</p>
      <Show when={message()}><p role="alert">{message()}</p></Show>
      <button type="submit" disabled={busy()}>Save disabled lane</button>
    </form>
  </div>;
}

export function TunnelControls(props: { clusterId: string; lane: TunnelLane; initData: string; saved: () => void }) {
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal("");
  async function apply(action: string, fields?: FormData) {
    if (busy() || !props.lane.managed) return;
    setBusy(true); setMessage("");
    try {
      await postOwner(`/clusters/${props.clusterId}/tunnels/${props.lane.id}`, props.initData,
        { action, revision: props.lane.managed.revision, destination_host: fields?.get("destination_host"), destination_port: Number(fields?.get("destination_port")) });
      props.saved();
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not update lane"); }
    finally { setBusy(false); }
  }
  return <Show when={props.lane.managed}>{(lane) => <fieldset disabled={busy()}>
    <legend>Lane controls</legend>
    <p>Desired state: {lane().enabled ? "Enabled" : "Disabled"} · TLS control port {lane().control_port}</p>
    <button type="button" onClick={() => void apply(lane().enabled ? "disable" : "enable")}>{lane().enabled ? "Disable" : "Enable"}</button>
    <Show when={lane().enabled} fallback={<button type="button" onClick={() => void apply("remove")}>Remove disabled lane</button>}>
      <button type="button" onClick={() => void apply("restart")}>Restart lane</button>
    </Show>
    <Show when={lane().listener_type === "tcp"}>
      <form onSubmit={(event) => { event.preventDefault(); void apply("destination", new FormData(event.currentTarget)); }}>
        <TcpDestination host={lane().destination_host} port={lane().destination_port} />
        <button type="submit">Save destination</button>
      </form>
    </Show>
    <Show when={message()}><p role="alert">{message()}</p></Show>
  </fieldset>}</Show>;
}
