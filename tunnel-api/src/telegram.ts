import { error, json } from "./http";
import type { Env, NodeRow, OwnerRow, TelegramUpdate } from "./models";
import { nodeManifest } from "./nodes";
import { audit, clusterByName, createEnrollmentGrant, nodeByName, ownerForTelegram } from "./store";
import { apiOrigin, equalSecret, filesOrigin, NODE_NAME, now, validNodeClass } from "./validation";

const helpText = [
  "TunnelOwnersBot commands",
  "/cluster_add <name>",
  "/cluster_list",
  "/server_add <cluster> <domestic|foreign> <server> [address]",
  "/server_attach <cluster> <domestic|foreign> <server>",
  "/server_list",
  "/install <server>",
  "Server names and cluster names use lowercase letters, numbers, and hyphens.",
].join("\n");

async function telegramReply(env: Env, chatId: number, text: string): Promise<void> {
  if (!env.TELEGRAM_BOT_TOKEN) return;
  await fetch(`https://api.telegram.org/bot${env.TELEGRAM_BOT_TOKEN}/sendMessage`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ chat_id: chatId, text, disable_web_page_preview: true }),
  });
}

async function addCluster(env: Env, owner: OwnerRow, args: string[]): Promise<string> {
  const name = args[0]?.toLowerCase();
  if (!name || !NODE_NAME.test(name)) return "Usage: /cluster_add <lowercase-name>";
  if (await clusterByName(env, owner.id, name)) return `Cluster ${name} already exists.`;

  const id = crypto.randomUUID();
  await env.DB.prepare("INSERT INTO clusters (id, owner_id, name, created_at) VALUES (?, ?, ?, ?)")
    .bind(id, owner.id, name, now())
    .run();
  await audit(env, "cluster.created", { name }, { ownerId: owner.id, clusterId: id });
  return `Created cluster ${name}.`;
}

async function listClusters(env: Env, owner: OwnerRow): Promise<string> {
  const rows = await env.DB.prepare("SELECT name FROM clusters WHERE owner_id = ? ORDER BY name")
    .bind(owner.id)
    .all<{ name: string }>();
  return rows.results.length ? `Clusters:\n${rows.results.map((row) => `- ${row.name}`).join("\n")}` : "No clusters yet.";
}

async function attachNode(env: Env, owner: OwnerRow, command: string, args: string[]): Promise<string> {
  const [clusterName, className, nodeName, address] = args;
  const cluster = clusterName ? await clusterByName(env, owner.id, clusterName.toLowerCase()) : null;
  const nodeClass = className?.toLowerCase();
  const name = nodeName?.toLowerCase();
  if (!cluster || !validNodeClass(nodeClass) || !name || !NODE_NAME.test(name)) {
    return `Usage: ${command} <cluster> <domestic|foreign> <server> ${command === "/server_add" ? "[address]" : ""}`;
  }

  let node = await nodeByName(env, owner.id, name);
  if (!node && command === "/server_attach") return `Server ${name} does not exist; add it first.`;
  if (!node) node = await createNode(env, owner, name, nodeClass, address);
  if (node.node_class !== nodeClass) {
    return `Server ${name} is already ${node.node_class}; node class cannot be changed through attach.`;
  }

  await env.DB.prepare(
    "INSERT OR IGNORE INTO cluster_nodes (cluster_id, node_id, role, created_at) VALUES (?, ?, ?, ?)",
  )
    .bind(cluster.id, node.id, nodeClass, now())
    .run();
  await audit(env, "cluster.node_attached", { node: name, role: nodeClass }, {
    ownerId: owner.id,
    nodeId: node.id,
    clusterId: cluster.id,
  });
  return `Server ${name} is a ${nodeClass} member of ${cluster.name}. Run /install ${name} on the server.`;
}

async function createNode(
  env: Env,
  owner: OwnerRow,
  name: string,
  nodeClass: NodeRow["node_class"],
  address?: string,
): Promise<NodeRow> {
  const id = crypto.randomUUID();
  await env.DB.prepare(
    `INSERT INTO nodes (id, owner_id, name, node_class, address, created_at)
     VALUES (?, ?, ?, ?, ?, ?)`,
  )
    .bind(id, owner.id, name, nodeClass, address ?? null, now())
    .run();
  await env.DB.prepare(
    "INSERT INTO node_documents (node_id, revision, manifest_json, updated_at) VALUES (?, ?, ?, ?)",
  )
    .bind(id, 0, nodeManifest(nodeClass), now())
    .run();
  return { id, name, node_class: nodeClass, address: address ?? null, status: "pending" };
}

async function listNodes(env: Env, owner: OwnerRow): Promise<string> {
  const rows = await env.DB.prepare(
    "SELECT name, node_class, address, status FROM nodes WHERE owner_id = ? ORDER BY name",
  )
    .bind(owner.id)
    .all<NodeRow>();
  return rows.results.length
    ? `Servers:\n${rows.results.map((row) => `- ${row.name} (${row.node_class}, ${row.status}${row.address ? `, ${row.address}` : ""})`).join("\n")}`
    : "No servers yet.";
}

async function installNode(env: Env, owner: OwnerRow, args: string[]): Promise<string> {
  const name = args[0]?.toLowerCase();
  const node = name ? await nodeByName(env, owner.id, name) : null;
  const memberships = node
    ? await env.DB.prepare("SELECT COUNT(*) AS count FROM cluster_nodes WHERE node_id = ?").bind(node.id).first<{ count: number }>()
    : null;
  if (!node || !memberships || memberships.count < 1) {
    return "Usage: /install <server>. The server must belong to at least one cluster.";
  }

  const grant = await createEnrollmentGrant(env, node);
  await audit(env, "node.enrollment_grant_created", { expires_in_seconds: env.ENROLLMENT_TTL_SECONDS }, {
    ownerId: owner.id,
    nodeId: node.id,
  });
  return `Run one command on ${node.name}. This grant is single-use and short-lived.\n\n${installCommand(env, node, grant)}`;
}

function installCommand(env: Env, node: NodeRow, grant: string): string {
  const publicInstaller = `${filesOrigin(env)}/install.sh`;
  const domesticInstaller = env.DOMESTIC_BOOTSTRAP_URL?.trim();
  const installer = node.node_class === "domestic" && domesticInstaller?.startsWith("https://")
    ? domesticInstaller
    : publicInstaller;
  const shared = `TUNNEL_AGENT_API=${apiOrigin(env)}/api TUNNEL_AGENT_ENROLLMENT=${grant}`;
  return [`curl -fsSL ${installer} | sudo env ${shared} sh`, `wget -qO- ${installer} | sudo env ${shared} sh`].join("\n\n");
}

async function commandResponse(env: Env, owner: OwnerRow, command: string | undefined, args: string[]): Promise<string> {
  switch (command) {
    case "/cluster_add": return addCluster(env, owner, args);
    case "/cluster_list": return listClusters(env, owner);
    case "/server_add":
    case "/server_attach": return attachNode(env, owner, command, args);
    case "/server_list": return listNodes(env, owner);
    case "/install": return installNode(env, owner, args);
    case "/start":
    case "/help": return helpText;
    default: return helpText;
  }
}

async function handleBotCommand(env: Env, update: TelegramUpdate): Promise<void> {
  const incoming = update.message;
  if (!incoming?.from || !incoming.text) return;
  const parts = incoming.text.trim().split(/\s+/);
  const command = parts.shift()?.split("@", 1)[0].toLowerCase();
  const owner = await ownerForTelegram(env, incoming.from.id, incoming.from.username);
  await telegramReply(env, incoming.chat.id, await commandResponse(env, owner, command, parts));
}

export async function handleTelegram(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  if (!equalSecret(request.headers.get("X-Telegram-Bot-Api-Secret-Token"), env.TELEGRAM_WEBHOOK_SECRET)) {
    return error("unauthorized", 401);
  }
  let update: TelegramUpdate;
  try {
    update = await request.json<TelegramUpdate>();
  } catch {
    return error("invalid Telegram update");
  }
  if (!Number.isInteger(update.update_id)) return error("invalid Telegram update");
  const recorded = await env.DB.prepare("INSERT OR IGNORE INTO telegram_updates (update_id, received_at) VALUES (?, ?)")
    .bind(update.update_id, now())
    .run();
  if (recorded.meta.changes === 0) return json({ ok: true });
  ctx.waitUntil(handleBotCommand(env, update));
  return json({ ok: true });
}
