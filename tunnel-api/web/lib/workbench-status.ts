import type { ClusterNode, TunnelLane } from "../data/types";
import { isFresh, laneState } from "./format";

export function nodeStatus(node: ClusterNode) {
  if (node.status === "revoked") return "Revoked";
  if (node.status === "pending") return "Pending";
  if (node.last_heartbeat_at === null) return "No report";
  return isFresh(node.heartbeat_expires_at) ? "Active" : "Stalled";
}

export function laneHealth(lane: TunnelLane) {
  if (lane.managed?.enabled === false) return "Disabled";
  if (!lane.selected) return "Not selected";
  const state = laneState(lane);
  if (state === "healthy") return "Healthy";
  if (state === "degraded") return "Degraded";
  return "No probe";
}

export function routeKind(lane: TunnelLane) {
  return lane.kind === "management" ? "Management" : "Customer";
}

export function listenerLabel(lane: TunnelLane) {
  const listener = lane.local_listener;
  if (!listener) return "";
  const port = listener.match(/:(\d+)$/)?.[0];
  const kind = lane.managed?.listener_type ?? listener.match(/^(\w+):\/\//)?.[1];
  return port ? [kind, port].filter(Boolean).join(" ") : listener;
}
