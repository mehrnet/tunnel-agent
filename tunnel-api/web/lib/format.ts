import { createSignal } from "solid-js";
import type { TunnelLane } from "../data/types";

const [currentTime, setCurrentTime] = createSignal(Date.now());
setInterval(() => setCurrentTime(Date.now()), 15000);

export function formatNumber(value: number, digits = 0): string {
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: digits }).format(value);
}

export function formatPercent(value: number): string {
  return `${formatNumber(value)}%`;
}

export function countryName(code: string): string {
  return typeof Intl.DisplayNames === "function" ? new Intl.DisplayNames("en", { type: "region" }).of(code) ?? code : code;
}

export function countryFlag(code: string): string {
  return /^[A-Z]{2}$/.test(code) ? String.fromCodePoint(...Array.from(code, (letter) => letter.charCodeAt(0) + 127397)) : "";
}

export function formatBytes(value: number): string {
  const units = ["B", "KB", "MB", "GB"];
  const index = Math.min(Math.floor(Math.log(Math.max(value, 1)) / Math.log(1024)), units.length - 1);
  return `${formatNumber(value / 1024 ** index, index === 0 ? 0 : 1)} ${units[index]}`;
}

export function formatRate(value: number): string {
  return `${formatBytes(value)}/s`;
}

export function formatMoment(value: number | null): string {
  if (!value) return "No report";
  const seconds = Math.max(0, ageSeconds(value));
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} hr ago`;
  return `${Math.floor(seconds / 86400)} days ago`;
}

export function formatProbe(value: number | null, freshUntil: number | null): string | null {
  if (value === null || !isFresh(freshUntil)) return null;
  return `${formatNumber(value)} ms`;
}

export function isFresh(freshUntil: number | null): boolean {
  return freshUntil !== null && ageSeconds(freshUntil) <= 0;
}

export function laneState(lane: TunnelLane): TunnelLane["state"] {
  return lane.selected && isFresh(lane.fresh_until) ? lane.state : "unknown";
}

export function laneStatus(lane: TunnelLane): string {
  if (lane.managed && !lane.managed.enabled) return "Disabled";
  if (!lane.selected) return "Not selected";
  switch (laneState(lane)) {
    case "healthy": return lane.probe_scope === "local_listener" ? "Listener reachable" : "Control reachable";
    case "degraded": return lane.probe_scope === "local_listener" ? "Listener unreachable" : "Control unreachable";
    case "unknown": return "No current probe";
  }
}

export function probeLabel(lane: TunnelLane): string {
  if (lane.probe_scope === "local_listener") return "Listener TCP probe";
  return lane.kind === "management" ? "TCP probe" : "Control TCP probe";
}

function ageSeconds(value: number): number {
  const timestamp = value < 1_000_000_000_000 ? value * 1000 : value;
  return Math.floor((currentTime() - timestamp) / 1000);
}
