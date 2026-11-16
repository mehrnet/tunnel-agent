import { issueDomesticCapsule } from "./capsules";
import { replaceNodeDocument } from "./documents";
import { nodeManifest } from "./nodes";
import { downloadTelegramDocument, answerCallback, sendInstallCommand } from "./telegram-client";
import { presentMenu, type MenuView } from "./telegram-menu";
import type { Env, NodeClass, NodeRow, OwnerRow, TelegramCallbackQuery, TelegramMessage, TelegramSessionRow } from "./models";
import { attachNodeToCluster, audit, clusterById, clusterByName, clusterMembershipCount, createClusterForOwner, createEnrollmentGrant, createNodeForOwner, nodeForOwnerById, nodeByName, ownerForTelegram, setTelegramSessionState, telegramSession } from "./store";
import { apiOrigin, filesOrigin, NODE_NAME, now, validNodeClass } from "./validation";

const NODE_ID = /^[0-9a-f-]{36}$/;

export async function handleMenuCallback(env: Env, callback: TelegramCallbackQuery): Promise<void> {
  const message = callback.message;
  if (!message) return answerCallback(env, callback.id, "This inline menu cannot be used here.");
  const owner = await ownerForTelegram(env, callback.from.id, callback.from.username);
  const session = await telegramSession(env, owner.id, message.chat.id);
  if (!session || session.menu_message_id !== message.message_id) {
    return answerCallback(env, callback.id, "This menu is stale. Send /start for a new menu.");
  }
  await answerCallback(env, callback.id);
  try {
    await applyCallback(env, owner, message.chat.id, session, callback.data ?? "");
  } catch (reason) {
    await presentMenu(env, owner, message.chat.id, session, { kind: "home", notice: displayError(reason) });
  }
}

export async function handleMenuMessage(env: Env, message: TelegramMessage): Promise<void> {
  if (!message.from) return;
  const owner = await ownerForTelegram(env, message.from.id, message.from.username);
  let session = await telegramSession(env, owner.id, message.chat.id);
  const text = message.text?.trim() ?? message.caption?.trim();
  if (text === "/start" || text === "/menu" || text === "/help") return showHome(env, owner, message.chat.id);
  if (!session) return showHome(env, owner, message.chat.id);

  try {
    if (session.mode === "cluster-create" && text) return createClusterFromInput(env, owner, message.chat.id, session, text);
    if (session.mode === "server-create" && text) return createServerFromInput(env, owner, message.chat.id, session, text);
    if (session.mode === "manifest-apply" && message.document) return applyManifestFromInput(env, owner, message.chat.id, session, message.document);
    await presentMenu(env, owner, message.chat.id, session, pendingPrompt(session));
  } catch (reason) {
    await presentMenu(env, owner, message.chat.id, session, { kind: "home", notice: displayError(reason) });
  }
}

async function applyCallback(
  env: Env,
  owner: OwnerRow,
  chatId: number,
  session: TelegramSessionRow,
  data: string,
): Promise<void> {
  const [action, value, extra] = data.split(":");
  if (extra !== undefined) throw new Error("Invalid menu action.");
  if (action === "home") return showHome(env, owner, chatId);
  if (action === "help") return showPrompt(env, owner, chatId, "Help", "Use the buttons to browse clusters and servers. When prompted, send the requested name, server details, or manifest file.");
  if (action === "clusters") return showHome(env, owner, chatId);
  if (action === "nodes") return showView(env, owner, chatId, { kind: "nodes" });
  if (action === "cluster-add") return setPrompt(env, owner, chatId, "cluster-create", {}, "New cluster", "Send the lowercase cluster name.");
  if (action === "cluster" && validId(value)) return showView(env, owner, chatId, { kind: "cluster", clusterId: value });
  if (action === "node" && validId(value)) return showView(env, owner, chatId, { kind: "node", nodeId: value });
  if (action === "newd" && validId(value)) return setServerPrompt(env, owner, chatId, value, "domestic");
  if (action === "newf" && validId(value)) return setServerPrompt(env, owner, chatId, value, "foreign");
  if (action === "attach" && validId(value)) return showAttach(env, owner, chatId, value);
  if (action === "attach-node" && validId(value)) return attachExistingNode(env, owner, chatId, session, value);
  if (action === "install" && validId(value)) return issueInstall(env, owner, chatId, session, value);
  if (action === "manifest" && validId(value)) return setManifestPrompt(env, owner, chatId, value);
  throw new Error("This menu action is no longer available.");
}

async function showHome(env: Env, owner: OwnerRow, chatId: number): Promise<void> {
  const session = await setTelegramSessionState(env, owner.id, chatId, "idle", {});
  await presentMenu(env, owner, chatId, session, { kind: "home" });
}

async function showView(env: Env, owner: OwnerRow, chatId: number, view: MenuView): Promise<void> {
  const active = await setTelegramSessionState(env, owner.id, chatId, "idle", {});
  await presentMenu(env, owner, chatId, active, view);
}

async function showPrompt(env: Env, owner: OwnerRow, chatId: number, title: string, detail: string): Promise<void> {
  const session = await setTelegramSessionState(env, owner.id, chatId, "idle", {});
  await presentMenu(env, owner, chatId, session, { kind: "prompt", title, detail });
}

async function setPrompt(env: Env, owner: OwnerRow, chatId: number, mode: string, payload: unknown, title: string, detail: string): Promise<void> {
  const session = await setTelegramSessionState(env, owner.id, chatId, mode, payload);
  await presentMenu(env, owner, chatId, session, { kind: "prompt", title, detail });
}

async function setServerPrompt(env: Env, owner: OwnerRow, chatId: number, clusterId: string, nodeClass: NodeClass): Promise<void> {
  if (!await clusterById(env, owner.id, clusterId)) throw new Error("That cluster no longer exists.");
  await setPrompt(env, owner, chatId, "server-create", { clusterId, nodeClass }, `New ${nodeClass} server`, "Send `server-name [address]`.");
}

async function showAttach(env: Env, owner: OwnerRow, chatId: number, clusterId: string): Promise<void> {
  if (!await clusterById(env, owner.id, clusterId)) throw new Error("That cluster no longer exists.");
  const session = await setTelegramSessionState(env, owner.id, chatId, "attach", { clusterId });
  await presentMenu(env, owner, chatId, session, { kind: "attach", clusterId });
}

async function attachExistingNode(env: Env, owner: OwnerRow, chatId: number, session: TelegramSessionRow, nodeId: string): Promise<void> {
  const clusterId = requiredPayloadId(session, "clusterId");
  const [cluster, node] = await Promise.all([clusterById(env, owner.id, clusterId), nodeForOwnerById(env, owner.id, nodeId)]);
  if (!cluster || !node) throw new Error("The selected server or cluster no longer exists.");
  await attachNodeToCluster(env, cluster.id, node.id, node.node_class);
  await audit(env, "cluster.node_attached", { node: node.name, role: node.node_class }, { ownerId: owner.id, nodeId: node.id, clusterId: cluster.id });
  const active = await setTelegramSessionState(env, owner.id, chatId, "idle", {});
  await presentMenu(env, owner, chatId, active, { kind: "cluster", clusterId, notice: `${node.name} attached.` });
}

async function issueInstall(env: Env, owner: OwnerRow, chatId: number, session: TelegramSessionRow, nodeId: string): Promise<void> {
  const node = await nodeForOwnerById(env, owner.id, nodeId);
  if (!node || await clusterMembershipCount(env, nodeId) < 1) throw new Error("This server must belong to a cluster before installation.");
  const command = await installCommand(env, owner, node);
  await sendInstallCommand(env, chatId, node.name, command);
  const active = await setTelegramSessionState(env, owner.id, chatId, "idle", {});
  await presentMenu(env, owner, chatId, active, { kind: "node", nodeId, notice: "A one-time installation command was sent." });
}

async function setManifestPrompt(env: Env, owner: OwnerRow, chatId: number, nodeId: string): Promise<void> {
  if (!await nodeForOwnerById(env, owner.id, nodeId)) throw new Error("That server no longer exists.");
  await setPrompt(env, owner, chatId, "manifest-apply", { nodeId }, "Apply desired state", "Attach the v3 UTF-8 JSON manifest file now.");
}

async function createClusterFromInput(env: Env, owner: OwnerRow, chatId: number, _session: TelegramSessionRow, text: string): Promise<void> {
  const name = text.toLowerCase();
  if (!NODE_NAME.test(name)) throw new Error("Cluster names use lowercase letters, numbers, and hyphens.");
  if (await clusterByName(env, owner.id, name)) throw new Error(`Cluster ${name} already exists.`);
  const cluster = await createClusterForOwner(env, owner.id, name);
  await audit(env, "cluster.created", { name }, { ownerId: owner.id, clusterId: cluster.id });
  const active = await setTelegramSessionState(env, owner.id, chatId, "idle", {});
  await presentMenu(env, owner, chatId, active, { kind: "cluster", clusterId: cluster.id, notice: `Created ${name}.` });
}

async function createServerFromInput(env: Env, owner: OwnerRow, chatId: number, session: TelegramSessionRow, text: string): Promise<void> {
  const [rawName, address, extra] = text.trim().split(/\s+/);
  const clusterId = requiredPayloadId(session, "clusterId");
  const nodeClass = payloadNodeClass(session);
  const name = rawName?.toLowerCase();
  if (!name || extra || !NODE_NAME.test(name)) throw new Error("Send `server-name [address]` using a lowercase server name.");
  if (!await clusterById(env, owner.id, clusterId)) throw new Error("That cluster no longer exists.");
  if (await nodeByName(env, owner.id, name)) throw new Error(`${name} already exists. Use Attach server from the cluster menu.`);
  const node = await createNodeForOwner(env, owner.id, name, nodeClass, address ?? null, nodeManifest(nodeClass));
  await attachNodeToCluster(env, clusterId, node.id, nodeClass);
  await audit(env, "cluster.node_attached", { node: name, role: nodeClass }, { ownerId: owner.id, nodeId: node.id, clusterId });
  const active = await setTelegramSessionState(env, owner.id, chatId, "idle", {});
  await presentMenu(env, owner, chatId, active, { kind: "node", nodeId: node.id, notice: `${name} was created and attached.` });
}

async function applyManifestFromInput(env: Env, owner: OwnerRow, chatId: number, session: TelegramSessionRow, document: TelegramMessage["document"]): Promise<void> {
  if (!document) throw new Error("Attach a JSON manifest file.");
  const nodeId = requiredPayloadId(session, "nodeId");
  const node = await nodeForOwnerById(env, owner.id, nodeId);
  if (!node) throw new Error("That server no longer exists.");
  const updated = await replaceNodeDocument(env, owner, node, await downloadTelegramDocument(env, document));
  const active = await setTelegramSessionState(env, owner.id, chatId, "idle", {});
  await presentMenu(env, owner, chatId, active, { kind: "node", nodeId, notice: `Applied revision ${updated.revision}.` });
}

async function installCommand(env: Env, owner: OwnerRow, node: NodeRow): Promise<string> {
  if (node.node_class === "domestic") {
    const capsule = await issueDomesticCapsule(env, node);
    await audit(env, "node.domestic_capsule_created", {}, { ownerId: owner.id, nodeId: node.id });
    const installer = env.DOMESTIC_BOOTSTRAP_URL?.trim().startsWith("https://") ? env.DOMESTIC_BOOTSTRAP_URL.trim() : `${filesOrigin(env)}/install.sh`;
    return `curl -fsSL ${installer} | sudo env TUNNEL_AGENT_CAPSULE=${capsule} sh`;
  }
  const grant = await createEnrollmentGrant(env, node);
  await audit(env, "node.enrollment_grant_created", { expires_in_seconds: env.ENROLLMENT_TTL_SECONDS }, { ownerId: owner.id, nodeId: node.id });
  return `curl -fsSL ${filesOrigin(env)}/install.sh | sudo env TUNNEL_AGENT_API=${apiOrigin(env)}/api TUNNEL_AGENT_ENROLLMENT=${grant} sh`;
}

function pendingPrompt(session: TelegramSessionRow): MenuView {
  if (session.mode === "cluster-create") return { kind: "prompt", title: "New cluster", detail: "Send the lowercase cluster name." };
  if (session.mode === "server-create") return { kind: "prompt", title: "New server", detail: "Send `server-name [address]`." };
  if (session.mode === "manifest-apply") return { kind: "prompt", title: "Apply desired state", detail: "Attach the v3 UTF-8 JSON manifest file now." };
  return { kind: "home" };
}

function payloadId(session: TelegramSessionRow, key: string, required: boolean): string | undefined {
  const value = payload(session)[key];
  if (typeof value === "string" && validId(value)) return value;
  if (required) throw new Error("The menu context expired. Open the menu again.");
  return undefined;
}

function payloadNodeClass(session: TelegramSessionRow): NodeClass {
  const value = payload(session).nodeClass;
  if (typeof value !== "string" || !validNodeClass(value)) throw new Error("The selected server class expired. Open the menu again.");
  return value;
}

function requiredPayloadId(session: TelegramSessionRow, key: string): string {
  const value = payloadId(session, key, false);
  if (!value) throw new Error("The menu context expired. Open the menu again.");
  return value;
}

function payload(session: TelegramSessionRow): Record<string, unknown> {
  try {
    const value: unknown = JSON.parse(session.payload_json);
    return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
  } catch {
    return {};
  }
}

function validId(value: string | undefined): value is string {
  return typeof value === "string" && NODE_ID.test(value);
}

function displayError(reason: unknown): string {
  return reason instanceof Error ? reason.message : "The requested operation failed.";
}
