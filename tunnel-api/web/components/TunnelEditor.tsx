import { For, Show, createSignal } from "solid-js";
import { postOwner } from "../api/client";
import type { ClusterNode, ClusterResponse, TunnelLane } from "../data/types";
import { OwnerDialog } from "./OwnerDialog";

function TcpDestination(props: { host?: string | null; port?: number | null }) {
  return <>
    <label>Destination from connector <input name="destination_host" required value={props.host ?? "127.0.0.1"} /></label>
    <label>Destination port <input name="destination_port" required type="number" min="1" max="65535" value={props.port ?? ""} /></label>
  </>;
}

function ServerField(props: { label: string; name: string; nodes: ClusterNode[] }) {
  return <label>{props.label} <select name={props.name} required>
    <option value="">Select a server</option>
    <For each={props.nodes}>{(node) => <option value={node.id}>{node.name} ({node.node_class})</option>}</For>
  </select></label>;
}

function TransportFields(props: { transport: string; kind: string; setTransport: (value: string) => void; setKind: (value: string) => void }) {
  return <fieldset>
    <legend>02 / Transport and ports</legend>
    <div class="detail-form-pair">
      <label>Transport <select value={props.transport} onChange={(event) => props.setTransport(event.currentTarget.value)}>
        <option value="wstunnel">WSTunnel (TLS)</option>
        <option value="hpnssh">HPNSSH</option>
        <option value="rathole">Rathole (Noise)</option>
      </select></label>
      <label>Listener type <select value={props.kind} onChange={(event) => props.setKind(event.currentTarget.value)}>
        <option value="tcp">TCP</option>
        <Show when={props.transport !== "rathole"}><option value="socks5">SOCKS5</option></Show>
      </select></label>
      <label>Loopback port <input name="listener_port" type="number" min="1024" max="65535" required /></label>
      <label>Control port <input name="control_port" type="number" min="1024" max="65535" required /></label>
    </div>
    <Show when={props.transport === "wstunnel"}>
      <label>TLS server name <input name="tls_server_name" placeholder="Required when dialing an IP address" maxlength="253" /></label>
      <Show when={props.kind === "tcp"}><label><input type="checkbox" name="forward_udp" /> Forward UDP too</label></Show>
    </Show>
  </fieldset>;
}

export function NewTunnel(props: { cluster: ClusterResponse; initData: string; close: () => void; saved: () => void }) {
  const [transport, setTransport] = createSignal("wstunnel");
  const [kind, setKind] = createSignal("tcp");
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal("");
  const nodes = () => props.cluster.nodes.filter((node) => node.status !== "revoked");
  function chooseTransport(selected: string) {
    setTransport(selected);
    if (selected === "rathole") setKind("tcp");
  }
  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (busy()) return;
    const fields = new FormData(event.currentTarget as HTMLFormElement);
    if (fields.get("source") === fields.get("listener")) return setMessage("Select two different servers.");
    setBusy(true); setMessage("");
    try {
      await postOwner(`/clusters/${props.cluster.cluster.id}/tunnels`, props.initData, {
        source_node_id: fields.get("source"), listener_node_id: fields.get("listener"), transport: transport(),
        listener_type: kind(), listener_port: Number(fields.get("listener_port")), control_port: Number(fields.get("control_port")),
        forward_udp: fields.has("forward_udp"),
        tls_server_name: fields.get("tls_server_name"),
        destination_host: fields.get("destination_host"), destination_port: Number(fields.get("destination_port")),
      });
      props.saved();
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not create lane"); }
    finally { setBusy(false); }
  }
  return <OwnerDialog class="node-popup lane-editor" title="new-lane-title" close={props.close}>
    <form onSubmit={(event) => void save(event)}>
      <header class="lane-editor__title">
        <div><p class="detail-kicker">LANE SETUP</p><h2 id="new-lane-title">New reverse tunnel</h2></div>
        <button type="button" onClick={() => props.close()}>Close</button>
      </header>
      <fieldset>
        <legend>01 / Endpoints</legend>
        <div class="detail-form-pair">
          <ServerField label="Connector" name="source" nodes={nodes()} />
          <ServerField label="Listening server" name="listener" nodes={nodes()} />
        </div>
        <p>The connector dials the listening server and forwards requests to the destination.</p>
      </fieldset>
      <TransportFields transport={transport()} kind={kind()} setTransport={chooseTransport} setKind={setKind} />
      <Show when={kind() === "tcp"}><fieldset>
        <legend>03 / Destination</legend>
        <div class="detail-form-pair"><TcpDestination /></div>
      </fieldset></Show>
      <p>{transport() === "wstunnel"
        ? "Saved disabled. Before enabling, install the listener's TLS certificate and key."
        : "Saved disabled. Both nodes must report their transport identity before you can enable the lane."}
        {" The connector must be able to reach the listening server's control port."}</p>
      <Show when={message()}><p role="alert">{message()}</p></Show>
      <button class="lane-editor__save" type="submit" disabled={busy()}>{busy() ? "Saving…" : "Save disabled lane"}</button>
    </form>
  </OwnerDialog>;
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
    <p>Desired state: {lane().enabled ? "Enabled" : "Disabled"} · Control port {lane().control_port}</p>
    <Show when={lane().tls_server_name}>{(name) => <p>TLS server name: {name()}</p>}</Show>
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
