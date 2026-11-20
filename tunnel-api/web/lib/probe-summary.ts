import type { ProbeJob } from "../data/types";

export function isProbeRunning(job: ProbeJob, now: number) {
  return job.cancelled_at === null && now < job.ends_at;
}

export function probeSummary(job: ProbeJob, now: number) {
  const running = isProbeRunning(job, now);
  const total = Math.ceil((job.ends_at - job.starts_at) / 30);
  const elapsed = Math.max(0, Math.min(total, Math.ceil(((job.cancelled_at ?? now) - job.starts_at) / 30)));
  const sequence = job.report?.sequence ?? 0;
  const awaiting = job.cancelled_at === null && !running && sequence < total && now - job.ends_at <= 180;
  const passed = job.report?.passed ?? 0;
  const failed = job.report?.failed ?? 0;
  const missed = Math.max(0, (running || awaiting ? sequence : elapsed) - passed - failed);
  const pending = Math.max(0, (job.cancelled_at === null ? total : elapsed) - passed - failed - missed);
  return { running, total, awaiting, passed, failed, missed, pending };
}

export function reportState(job: ProbeJob, now: number) {
  if (job.cancelled_at !== null) return { title: "Cancelled", tone: "unknown" };
  const { running, awaiting, failed, missed } = probeSummary(job, now);
  if (awaiting) return { title: "Awaiting final report", tone: "unknown" };
  if (running) {
    if (!job.reported_at) return { title: "Awaiting first report", tone: "unknown" };
    if (now - job.reported_at > 180) return { title: "Report stale", tone: "degraded" };
    return { title: "Running", tone: "active" };
  }
  if (!job.report) return { title: "No report received", tone: "unknown" };
  if (failed || missed) return { title: "Completed with issues", tone: "degraded" };
  return { title: "All checks passed", tone: "healthy" };
}
