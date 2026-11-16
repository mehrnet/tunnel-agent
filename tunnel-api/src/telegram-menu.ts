import { deleteMenu, editMenu, sendMenu, type TelegramMenu } from "./telegram-client";
import type { ClusterRow, Env, NodeRow, OwnerRow, TelegramSessionRow } from "./models";
import { clusterById, clustersForOwner, nodeDocument, nodeForOwnerById, nodesForCluster, nodesForOwner, setTelegramMenu, unattachedNodesForCluster } from "./store";
import { now } from "./validation";

const MENU_TTL_SECONDS = 10 * 60;
const MAX_CLUSTER_ROWS = 12;
const MAX_NODE_ROWS = 24;

export type MenuView =
  | { kind: "home"; notice?: string }
  | { kind: "cluster"; clusterId: string; notice?: string }
  | { kind: "nodes"; notice?: string }
  | { kind: "node"; nodeId: string; notice?: string }
  | { kind: "attach"; clusterId: string }
  | { kind: "prompt"; title: string; detail: string };

export async function presentMenu(
  env: Env,
  owner: OwnerRow,
  chatId: number,
  session: TelegramSessionRow,
  view: MenuView,
): Promise<void> {
  const menu = await buildMenu(env, owner, view);
  if (menu.text.length > 4096) throw new Error("menu text exceeds Telegram's limit");
  if (recentMenu(session)) {
    const edited = await editMenu(env, chatId, session.menu_message_id!, menu);
    if (edited) return;
  }
  if (session.menu_message_id !== null) await deleteMenu(env, chatId, session.menu_message_id);
  const sent = await sendMenu(env, chatId, menu);
  await setTelegramMenu(env, owner.id, chatId, sent.message_id, sent.date);
}

async function buildMenu(env: Env, owner: OwnerRow, view: MenuView): Promise<TelegramMenu> {
  switch (view.kind) {
    case "home": return homeMenu(await clustersForOwner(env, owner.id), view.notice);
    case "cluster": return clusterMenu(env, owner, view);
    case "nodes": return nodesMenu(await nodesForOwner(env, owner.id), view.notice);
    case "node": return nodeMenu(env, owner, view);
    case "attach": return attachMenu(env, owner, view.clusterId);
    case "prompt": return promptMenu(view.title, view.detail);
  }
}

function homeMenu(clusters: ClusterRow[], notice?: string): TelegramMenu {
  const rows = clusters.slice(0, MAX_CLUSTER_ROWS).flatMap((cluster) => [
    [button(cluster.name, `cluster:${cluster.id}`)],
    [button("Add domestic", `newd:${cluster.id}`), button("Add foreign", `newf:${cluster.id}`)],
  ]);
  const suffix = clusters.length > MAX_CLUSTER_ROWS ? `\nShowing the first ${MAX_CLUSTER_ROWS} clusters.` : "";
  return {
    text: ["Tunnel Agent", notice, clusters.length ? "Choose a cluster or add a server directly." : "Create a cluster to add its first server."]
      .filter(Boolean).join("\n\n") + suffix,
    keyboard: [...rows, [button("Servers", "nodes"), button("Add cluster", "cluster-add")], [button("Help", "help")]],
  };
}

async function clusterMenu(env: Env, owner: OwnerRow, view: Extract<MenuView, { kind: "cluster" }>): Promise<TelegramMenu> {
  const cluster = await clusterById(env, owner.id, view.clusterId);
  if (!cluster) return homeMenu(await clustersForOwner(env, owner.id), "That cluster no longer exists.");
  const nodes = await nodesForCluster(env, owner.id, cluster.id);
  const nodeRows = nodes.slice(0, MAX_NODE_ROWS).map((node) => [button(node.name, `node:${node.id}`)]);
  const summary = nodes.length ? nodes.map(nodeSummary).join("\n") : "No servers are attached.";
  return {
    text: [`Cluster: ${cluster.name}`, view.notice, summary].filter(Boolean).join("\n\n"),
    keyboard: [
      ...nodeRows,
      [button("Add domestic", `newd:${cluster.id}`), button("Add foreign", `newf:${cluster.id}`)],
      [button("Attach server", `attach:${cluster.id}`), button("Home", "home")],
    ],
  };
}

function nodesMenu(nodes: NodeRow[], notice?: string): TelegramMenu {
  const rows = nodes.slice(0, MAX_NODE_ROWS).map((node) => [button(node.name, `node:${node.id}`)]);
  const suffix = nodes.length > MAX_NODE_ROWS ? `\nShowing the first ${MAX_NODE_ROWS} servers.` : "";
  return {
    text: ["Servers", notice, nodes.length ? "Select a server." : "No servers yet."] .filter(Boolean).join("\n\n") + suffix,
    keyboard: [...rows, [button("Home", "home")]],
  };
}

async function nodeMenu(env: Env, owner: OwnerRow, view: Extract<MenuView, { kind: "node" }>): Promise<TelegramMenu> {
  const node = await nodeForOwnerById(env, owner.id, view.nodeId);
  if (!node) return homeMenu(await clustersForOwner(env, owner.id), "That server no longer exists.");
  const document = await nodeDocument(env, node.id);
  const text = [
    `Server: ${node.name}`,
    `Class: ${node.node_class}`,
    `Status: ${node.status}`,
    `Address: ${node.address ?? "not set"}`,
    `Desired-state revision: ${document?.revision ?? "missing"}`,
    view.notice,
  ].filter(Boolean).join("\n");
  return {
    text,
    keyboard: [[button("Installation command", `install:${node.id}`), button("Apply manifest", `manifest:${node.id}`)], [button("Back", "nodes")]],
  };
}

async function attachMenu(env: Env, owner: OwnerRow, clusterId: string): Promise<TelegramMenu> {
  const cluster = await clusterById(env, owner.id, clusterId);
  if (!cluster) return homeMenu(await clustersForOwner(env, owner.id), "That cluster no longer exists.");
  const nodes = await unattachedNodesForCluster(env, owner.id, clusterId);
  return {
    text: nodes.length ? `Attach server to ${cluster.name}` : `All of your servers are already attached to ${cluster.name}.`,
    keyboard: nodes.length
      ? [...nodes.slice(0, MAX_NODE_ROWS).map((node) => [button(`${node.name} (${node.node_class})`, `attach-node:${node.id}`)]), [button("Back", `cluster:${clusterId}`)]]
      : [[button("Back", `cluster:${clusterId}`)]],
  };
}

function promptMenu(title: string, detail: string): TelegramMenu {
  return { text: `${title}\n\n${detail}`, keyboard: [[button("Cancel", "home")]] };
}

function button(text: string, callbackData: string) {
  return { text, callback_data: callbackData };
}

function nodeSummary(node: NodeRow): string {
  return `- ${node.name}: ${node.node_class}, ${node.status}`;
}

function recentMenu(session: TelegramSessionRow): boolean {
  return session.menu_message_id !== null
    && session.menu_created_at !== null
    && session.menu_created_at >= now() - MENU_TTL_SECONDS;
}
