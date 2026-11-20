import { For, Show, createSignal } from "solid-js";
import { postOwner } from "../api/client";
import type { ClusterResponse, TunnelLane } from "../data/types";
import { isProbeRunning } from "../lib/probe-summary";
import { ProbeReport } from "./ProbeReport";

const durations = [
  [10, "10 minutes"], [30, "30 minutes"], [60, "1 hour"], [120, "2 hours"],
  [180, "3 hours"], [360, "6 hours"], [720, "12 hours"], [1440, "24 hours"],
] as const;

function ProbeForm(props: { hostname: string; preferredStatus: number; busy: boolean; start: (body: Record<string, unknown>) => void }) {
  const [duration, setDuration] = createSignal("30");
  const [download, setDownload] = createSignal(false);
  function submit(event: SubmitEvent) {
    event.preventDefault();
    const data = new FormData(event.currentTarget as HTMLFormElement);
    props.start({
      duration: Number(data.get("minutes") ?? duration()) * 60,
      hostname: data.get("hostname"), path: data.get("path"),
      expected_status: Number(data.get("expected_status")), download: download(),
    });
  }
  return <form class="probe-form" onSubmit={submit}>
    <div class="detail-form-pair">
      <label>Window <select value={duration()} onChange={(event) => setDuration(event.currentTarget.value)}>
        <For each={durations}>{([minutes, label]) => <option value={minutes}>{label}</option>}</For>
        <option value="custom">Custom</option>
      </select></label>
      <Show when={duration() === "custom"}><label>Minutes <input name="minutes" type="number" min="10" max="1440" required /></label></Show>
    </div>
    <label>TLS hostname <input name="hostname" required value={props.hostname} placeholder="service.example.com" maxlength="253" /></label>
    <div class="detail-form-pair">
      <label>HTTPS path <input name="path" required value="/" maxlength="160" /></label>
      <Show when={download()} fallback={<label>Expected status <input name="expected_status" type="number" min="200" max="599" required value={props.preferredStatus} /></label>}>
        <label>Expected status <select name="expected_status" value="206"><option value="200">200 OK</option><option value="206">206 Partial Content</option></select></label>
      </Show>
    </div>
    <label class="probe-form__check"><input type="checkbox" checked={download()} onChange={(event) => setDownload(event.currentTarget.checked)} />
      Read a 64 KiB download sample</label>
    <p>One HTTPS check every 30 seconds through this lane. It does not verify VLESS authentication.</p>
    <button class="detail-primary" type="submit" disabled={props.busy}>{props.busy ? "Starting…" : "Start probe"}</button>
  </form>;
}

export function ProbePanel(props: { cluster: ClusterResponse; lane: TunnelLane; initData: string; refresh: () => void }) {
  const [building, setBuilding] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal("");
  const jobs = () => (props.cluster.probe_jobs ?? []).filter((job) => job.lane_id === props.lane.id);
  const route = () => props.cluster.public_ingress?.find((item) =>
    item.lane_id === props.lane.id || item.fallback_lane_id === props.lane.id);
  const unavailable = () => {
    if (!props.lane.managed?.enabled) return "Enable this lane to start a probe.";
    const active = (props.cluster.probe_jobs ?? []).some(
      (job) => job.node_id === props.lane.peer_node_id && isProbeRunning(job, props.cluster.generated_at),
    );
    return active ? "The listening server already has a running probe." : "";
  };

  async function submit(body: Record<string, unknown>) {
    if (busy() || !props.initData) return;
    setBusy(true); setMessage("");
    try {
      await postOwner(`/clusters/${props.cluster.cluster.id}/probes`, props.initData, body);
      if (body.action === "start") setBuilding(false);
      setMessage(body.action === "start" ? "Probe scheduled." : "Cancellation requested.");
      props.refresh();
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not save probe job."); }
    finally { setBusy(false); }
  }

  return <Show when={props.lane.managed?.listener_type === "tcp" || jobs().length}>
    <section class="probe-panel" aria-label="Reliability probes">
      <header class="probe-panel__heading">
        <div><p class="detail-kicker">END-TO-END</p><h3>Reliability probes</h3></div>
        <Show when={!unavailable() && props.initData}><button type="button" aria-expanded={building()} onClick={() => { setMessage(""); setBuilding(!building()); }}>
          {building() ? "Close setup" : "New probe"}</button></Show>
      </header>
      <Show when={building() && !unavailable()}><ProbeForm busy={busy()}
        hostname={route()?.tls_server_name ?? ""} preferredStatus={route() ? 404 : 200}
        start={(fields) => void submit({ ...fields, action: "start", lane_id: props.lane.id })} /></Show>
      <Show when={message()}><p class="probe-panel__message" role="status">{message()}</p></Show>
      <Show when={jobs()[0]} fallback={<p class="probe-panel__hint">No reliability runs for this lane yet.</p>}>
        {(job) => <ProbeReport job={job()} now={props.cluster.generated_at} busy={busy() || !props.initData}
          cancel={() => void submit({ action: "cancel", id: job().id })} />}
      </Show>
      <Show when={unavailable()}><p class="probe-panel__hint">{unavailable()}</p></Show>
      <Show when={jobs().length > 1}><details class="node-lifecycle">
        <summary>Run history ({jobs().length - 1})</summary>
        <For each={jobs().slice(1)}>{(job) => <ProbeReport job={job} now={props.cluster.generated_at} busy={busy() || !props.initData}
          cancel={() => void submit({ action: "cancel", id: job.id })} />}</For>
      </details></Show>
    </section>
  </Show>;
}
