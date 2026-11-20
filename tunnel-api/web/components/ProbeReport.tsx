import { Show } from "solid-js";
import type { ProbeJob } from "../data/types";
import { formatMoment, formatNumber } from "../lib/format";
import { isProbeRunning, probeSummary, reportState } from "../lib/probe-summary";

function CheckCounts(props: { job: ProbeJob; now: number }) {
  const counts = () => probeSummary(props.job, props.now);
  const scale = (count: number) => `${100 * count / counts().total}%`;
  return <>
    <div class="probe-report__bar" aria-hidden="true">
      <i class="probe-report__pass" style={{ width: scale(counts().passed) }} />
      <i class="probe-report__fail" style={{ width: scale(counts().failed) }} />
      <i class="probe-report__miss" style={{ width: scale(counts().missed) }} />
    </div>
    <div class="probe-report__counts">
      <span><b>{counts().passed}</b> Passed</span>
      <span><b>{counts().failed}</b> Failed</span>
      <span><b>{counts().missed}</b> Missed</span>
      <span><b>{counts().pending}</b> Pending</span>
    </div>
  </>;
}

export function ProbeReport(props: { job: ProbeJob; now: number; cancel: () => void; busy: boolean }) {
  const state = () => reportState(props.job, props.now);
  const minutes = () => formatNumber((props.job.ends_at - props.job.starts_at) / 60, 1);
  return <article class="probe-report">
    <header class="probe-report__head">
      <strong class={`detail-status detail-status--${state().tone}`}>{state().title}</strong>
      <span>{minutes()} min / {props.job.target.download ? "64 KiB sample" : "HTTPS check"}</span>
    </header>
    <code class="probe-report__target">{props.job.target.hostname}{props.job.target.path}</code>
    <CheckCounts job={props.job} now={props.now} />
    <Show when={props.job.report}>{(sample) => <div class="probe-report__facts">
      <Show when={sample().passed > 0}><span>Request avg / max <b>{formatNumber(sample().latency_total_ms / sample().passed)} / {formatNumber(sample().latency_max_ms)} ms</b></span></Show>
      <Show when={sample().download_ms > 0}><span>Sample rate incl. setup <b>{formatNumber(sample().bytes / sample().download_ms / 1000, 2)} MB/s</b></span></Show>
      <span>Longest failure streak <b>{sample().longest_streak}</b></span>
      <Show when={sample().last_error}>{(failure) => <span>Last error <b>{failure().replaceAll("_", " ")}</b></span>}</Show>
    </div>}</Show>
    <footer class="probe-report__foot">
      <span>{props.job.reported_at ? `Reported ${formatMoment(props.job.reported_at)}` : `Started ${formatMoment(props.job.starts_at)}`}</span>
      <Show when={isProbeRunning(props.job, props.now)}><button type="button" disabled={props.busy} onClick={() => props.cancel()}>Cancel run</button></Show>
    </footer>
  </article>;
}
