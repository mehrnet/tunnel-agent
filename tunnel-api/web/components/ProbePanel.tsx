import { For, Show, createSignal } from "solid-js";
import { postOwner } from "../api/client";
import type { ClusterResponse, ProbeJob, TunnelLane } from "../data/types";
import { formatMoment } from "../lib/format";

const durations = [
  [10, "10 minutes"], [30, "30 minutes"], [60, "1 hour"], [120, "2 hours"],
  [180, "3 hours"], [360, "6 hours"], [720, "12 hours"], [1440, "24 hours"],
] as const;

function active(job: ProbeJob, now: number) {
  return job.cancelled_at === null && now < job.ends_at;
}

function Report(props: { job: ProbeJob; now: number; cancel: () => void; busy: boolean }) {
  const report = () => props.job.report;
  const checked = () => (report()?.passed ?? 0) + (report()?.failed ?? 0);
  const total = () => {
    const stop = props.job.cancelled_at ?? props.job.ends_at;
    return Math.max(0, Math.ceil((stop - props.job.starts_at) / 30));
  };
  const live = () => active(props.job, props.now);
  const missing = () => Math.max(0, (live() ? report()?.sequence ?? 0 : total()) - checked());
  const status = () => {
    if (props.job.cancelled_at !== null) return "Cancelled";
    if (!live()) return "Window ended";
    if (!props.job.reported_at) return "Waiting for report";
    return props.now - props.job.reported_at > 180 ? "Report stale" : "Running";
  };
  return <div class="detail-list">
    <p><strong>{status()}</strong> · {props.job.target.hostname}{props.job.target.path}</p>
    <p>{report()?.passed ?? 0} passed / {report()?.failed ?? 0} failed / {missing()} missed · {checked()}/{total()} checked</p>
    <Show when={report()}>{(sample) => <>
      <Show when={sample().passed > 0}>
        <p>Request time: {Math.round(sample().latency_total_ms / sample().passed)} ms avg / {sample().latency_max_ms} ms max</p>
      </Show>
      <p>Longest failure streak: {sample().longest_streak} checks
        {sample().last_error ? ` · Last check: ${sample().last_error?.replaceAll("_", " ")}` : ""}</p>
      <Show when={sample().download_ms > 0}>
        <p>Sample rate: {(sample().bytes / sample().download_ms / 1000).toFixed(2)} MB/s including request setup</p>
      </Show>
      <p>Last report {formatMoment(props.job.reported_at)}</p>
    </>}</Show>
    <Show when={live()}><button type="button" disabled={props.busy} onClick={() => props.cancel()}>Cancel probe</button></Show>
  </div>;
}

export function ProbePanel(props: { cluster: ClusterResponse; lane: TunnelLane; initData: string; refresh: () => void }) {
  const [duration, setDuration] = createSignal("30");
  const [download, setDownload] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal("");
  const jobs = () => (props.cluster.probe_jobs ?? []).filter((job) => job.lane_id === props.lane.id);
  const route = () => props.cluster.public_ingress?.find((item) =>
    item.lane_id === props.lane.id || item.fallback_lane_id === props.lane.id);
  const canStart = () => props.lane.managed?.enabled && !(props.cluster.probe_jobs ?? []).some(
    (job) => job.node_id === props.lane.peer_node_id && active(job, props.cluster.generated_at),
  );

  async function submit(body: Record<string, unknown>) {
    if (busy() || !props.initData) return;
    setBusy(true); setMessage("");
    try {
      await postOwner(`/clusters/${props.cluster.cluster.id}/probes`, props.initData, body);
      props.refresh();
    } catch (error) { setMessage(error instanceof Error ? error.message : "Could not save probe job"); }
    finally { setBusy(false); }
  }

  return <Show when={props.lane.managed?.listener_type === "tcp" || jobs().length}>
    <details class="node-lifecycle">
      <summary>Reliability probes</summary>
      <Show when={canStart()}>
        <form onSubmit={(event) => {
          event.preventDefault();
          const data = new FormData(event.currentTarget);
          void submit({
            action: "start", lane_id: props.lane.id, duration: Number(data.get("minutes") ?? duration()) * 60,
            hostname: data.get("hostname"), path: data.get("path"),
            expected_status: Number(data.get("expected_status")), download: download(),
          });
        }}>
          <label>Duration <select value={duration()} onChange={(event) => setDuration(event.currentTarget.value)}>
            <For each={durations}>{([minutes, label]) => <option value={minutes}>{label}</option>}</For>
            <option value="custom">Custom</option>
          </select></label>
          <Show when={duration() === "custom"}>
            <label>Minutes <input name="minutes" type="number" min="10" max="1440" required /></label>
          </Show>
          <label>TLS hostname <input name="hostname" required value={route()?.tls_server_name ?? ""} maxlength="253" /></label>
          <label>HTTPS path <input name="path" required value="/" maxlength="160" /></label>
          <label><input type="checkbox" checked={download()} onChange={(event) => setDownload(event.currentTarget.checked)} /> Read a 64 KiB download sample</label>
          <label>Expected HTTP status <input name="expected_status" type="number" min="200" max="599" required
            value={download() ? 206 : route() ? 404 : 200} /></label>
          <p>One check every 30 seconds through the lane. This checks HTTPS, not VLESS authentication.</p>
          <button type="submit" disabled={busy()}>Start probe</button>
        </form>
      </Show>
      <For each={jobs()}>{(job) => <Report job={job} now={props.cluster.generated_at} busy={busy()}
        cancel={() => void submit({ action: "cancel", id: job.id })} />}</For>
      <Show when={message()}><p role="alert">{message()}</p></Show>
    </details>
  </Show>;
}
