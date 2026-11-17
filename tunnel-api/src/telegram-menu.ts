import { deleteMenu, editMenu, sendMenu, type TelegramButton, type TelegramMenu } from "./telegram-client";
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
  | { kind: "node"; nodeId: string; returnClusterId?: string; notice?: string }
  | { kind: "attach"; clusterId: string }
  | { kind: "prompt"; title: string; detail: string };

export async function presentMenu(
  env: Env,
  owner: OwnerRow,
  chatId: number,
  session: TelegramSessionRow,
  view: MenuView,
): Promise<void> {
  const menu = await buildCheckedMenu(env, owner, view);
  if (recentMenu(session)) {
    const edited = await editMenu(env, chatId, session.menu_message_id!, menu);
    if (edited) return;
  }
  if (session.menu_message_id !== null) await deleteMenu(env, chatId, session.menu_message_id);
  const sent = await sendMenu(env, chatId, menu);
  await setTelegramMenu(env, owner.id, chatId, sent.message_id, sent.date);
}

export async function sendFreshMenu(
  env: Env,
  owner: OwnerRow,
  chatId: number,
  view: MenuView,
): Promise<void> {
  const menu = await buildCheckedMenu(env, owner, view);
  const sent = await sendMenu(env, chatId, menu);
  await setTelegramMenu(env, owner.id, chatId, sent.message_id, sent.date);
}

async function buildCheckedMenu(env: Env, owner: OwnerRow, view: MenuView): Promise<TelegramMenu> {
  const menu = await buildMenu(env, owner, view);
  if (menu.text.length > 4096) throw new Error("متن منو از حد مجاز تلگرام طولانی‌تر است.");
  return menu;
}

async function buildMenu(env: Env, owner: OwnerRow, view: MenuView): Promise<TelegramMenu> {
  switch (view.kind) {
    case "home": return homeMenu(env, await clustersForOwner(env, owner.id), view.notice);
    case "cluster": return clusterMenu(env, owner, view);
    case "nodes": return nodesMenu(await nodesForOwner(env, owner.id), view.notice);
    case "node": return nodeMenu(env, owner, view);
    case "attach": return attachMenu(env, owner, view.clusterId);
    case "prompt": return promptMenu(view.title, view.detail);
  }
}

function homeMenu(env: Env, clusters: ClusterRow[], notice?: string): TelegramMenu {
  const rows = clusters.slice(0, MAX_CLUSTER_ROWS).flatMap((cluster) => [
    [button(`📦 ${cluster.name}`, `cluster:${cluster.id}`)],
    [button("🇮🇷 افزودن داخلی", `newd:${cluster.id}`), button("🌍 افزودن خارجی", `newf:${cluster.id}`)],
  ]);
  const suffix = clusters.length > MAX_CLUSTER_ROWS ? `\nفقط ${MAX_CLUSTER_ROWS} خوشه اول نمایش داده می‌شوند.` : "";
  return {
    text: ["🧭 تونل ایجنت", notice, clusters.length ? "یک خوشه را انتخاب کنید یا سرور جدید را مستقیماً اضافه کنید." : "ابتدا یک خوشه بسازید تا نخستین سرور آن را اضافه کنید."]
      .filter(Boolean).join("\n\n") + suffix,
    keyboard: [
      ...rows,
      [webAppButton("📡 نقشه زنده", `${env.API_ORIGIN.replace(/\/$/, "")}/web/`)],
      [button("🖥 سرورها", "nodes"), button("➕ ساخت خوشه", "cluster-add")],
      [button("ℹ️ راهنما", "help")],
    ],
  };
}

async function clusterMenu(env: Env, owner: OwnerRow, view: Extract<MenuView, { kind: "cluster" }>): Promise<TelegramMenu> {
  const cluster = await clusterById(env, owner.id, view.clusterId);
  if (!cluster) return homeMenu(env, await clustersForOwner(env, owner.id), "❌ خوشه انتخاب‌شده دیگر وجود ندارد.");
  const nodes = await nodesForCluster(env, owner.id, cluster.id);
  const nodeRows = nodes.slice(0, MAX_NODE_ROWS).map((node) => [button(node.name, `node:${node.id}`)]);
  const summary = nodes.length ? nodes.map(nodeSummary).join("\n") : "سروری به این خوشه متصل نیست.";
  return {
    text: [`📦 خوشه: ${cluster.name}`, view.notice, summary].filter(Boolean).join("\n\n"),
    keyboard: [
      ...nodeRows,
      [button("🇮🇷 افزودن داخلی", `newd:${cluster.id}`), button("🌍 افزودن خارجی", `newf:${cluster.id}`)],
      [button("🔗 اتصال سرور موجود", `attach:${cluster.id}`), button("🏠 خانه", "home")],
    ],
  };
}

function nodesMenu(nodes: NodeRow[], notice?: string): TelegramMenu {
  const rows = nodes.slice(0, MAX_NODE_ROWS).map((node) => [button(node.name, `node:${node.id}`)]);
  const suffix = nodes.length > MAX_NODE_ROWS ? `\nفقط ${MAX_NODE_ROWS} سرور اول نمایش داده می‌شوند.` : "";
  return {
    text: ["🖥 سرورها", notice, nodes.length ? "یک سرور را انتخاب کنید." : "هنوز سروری ثبت نشده است."] .filter(Boolean).join("\n\n") + suffix,
    keyboard: [...rows, [button("🏠 خانه", "home")]],
  };
}

async function nodeMenu(env: Env, owner: OwnerRow, view: Extract<MenuView, { kind: "node" }>): Promise<TelegramMenu> {
  const node = await nodeForOwnerById(env, owner.id, view.nodeId);
  if (!node) return homeMenu(env, await clustersForOwner(env, owner.id), "❌ سرور انتخاب‌شده دیگر وجود ندارد.");
  const document = await nodeDocument(env, node.id);
  const text = [
    `🖥 سرور: ${node.name}`,
    `🧩 نوع: ${menuNodeClassLabel(node.node_class)}`,
    `📡 وضعیت: ${nodeStatusLabel(node)}`,
    heartbeatLabel(node),
    `📍 نشانی: ${node.address ?? "ثبت نشده"}`,
    `🧾 نسخه وضعیت مطلوب: ${document?.revision ?? "ثبت نشده"}`,
    view.notice,
  ].filter(Boolean).join("\n");
  const back = view.returnClusterId ? `cluster:${view.returnClusterId}` : "nodes";
  return {
    text,
    keyboard: [[button("📥 دستور نصب", `install:${node.id}`), button("📝 اعمال تنظیمات", `manifest:${node.id}`)], [button("⬅️ بازگشت", back)]],
  };
}

async function attachMenu(env: Env, owner: OwnerRow, clusterId: string): Promise<TelegramMenu> {
  const cluster = await clusterById(env, owner.id, clusterId);
  if (!cluster) return homeMenu(env, await clustersForOwner(env, owner.id), "❌ خوشه انتخاب‌شده دیگر وجود ندارد.");
  const nodes = await unattachedNodesForCluster(env, owner.id, clusterId);
  return {
    text: nodes.length ? `🔗 اتصال سرور به ${cluster.name}` : `✅ همه سرورهای شما به ${cluster.name} متصل هستند.`,
    keyboard: nodes.length
      ? [...nodes.slice(0, MAX_NODE_ROWS).map((node) => [button(`${node.name} (${menuNodeClassLabel(node.node_class)})`, `attach-node:${node.id}`)]), [button("⬅️ بازگشت", `cluster:${clusterId}`)]]
      : [[button("⬅️ بازگشت", `cluster:${clusterId}`)]],
  };
}

function promptMenu(title: string, detail: string): TelegramMenu {
  return { text: `${title}\n\n${detail}`, keyboard: [[button("✖️ لغو", "home")]] };
}

function button(text: string, callbackData: string): TelegramButton {
  return { text, callback_data: callbackData };
}

function webAppButton(text: string, url: string): TelegramButton {
  return { text, web_app: { url } };
}

function nodeSummary(node: NodeRow): string {
  return `• ${node.name}: ${menuNodeClassLabel(node.node_class)}، ${nodeStatusLabel(node)}${heartbeatAge(node)}`;
}

function menuNodeClassLabel(nodeClass: NodeRow["node_class"]): string {
  return nodeClass === "domestic" ? "داخلی" : "خارجی";
}

function nodeStatusLabel(node: NodeRow): string {
  if (node.status === "enrolled") return "فعال";
  if (node.status === "revoked") return "لغوشده";
  if (node.node_class !== "domestic") return "در انتظار نصب";
  if (node.last_heartbeat_at === null) return "منتظر نخستین پینگ رله";
  return now() - node.last_heartbeat_at <= 5 * 60 ? "فعال از مسیر رله" : "پینگ رله قدیمی";
}

function heartbeatLabel(node: NodeRow): string | undefined {
  if (node.node_class !== "domestic") return undefined;
  if (node.last_heartbeat_at === null) return "🫀 آخرین پینگ رله: هنوز دریافت نشده";
  return `🫀 آخرین پینگ رله: ${relativeAge(node.last_heartbeat_at)} پیش`;
}

function heartbeatAge(node: NodeRow): string {
  return node.node_class === "domestic" && node.last_heartbeat_at !== null
    ? `، پینگ ${relativeAge(node.last_heartbeat_at)} پیش`
    : "";
}

function relativeAge(timestamp: number): string {
  const seconds = Math.max(0, now() - timestamp);
  if (seconds < 60) return "کمتر از یک دقیقه";
  if (seconds < 60 * 60) return `${Math.floor(seconds / 60)} دقیقه`;
  if (seconds < 24 * 60 * 60) return `${Math.floor(seconds / (60 * 60))} ساعت`;
  return `${Math.floor(seconds / (24 * 60 * 60))} روز`;
}

function recentMenu(session: TelegramSessionRow): boolean {
  return session.menu_message_id !== null
    && session.menu_created_at !== null
    && session.menu_created_at >= now() - MENU_TTL_SECONDS;
}
