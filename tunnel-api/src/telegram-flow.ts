import { issueDomesticCapsule } from "./capsules";
import { replaceNodeDocument } from "./documents";
import { nodeManifest } from "./nodes";
import { downloadTelegramDocument, answerCallback, sendInstallCommand } from "./telegram-client";
import { presentMenu, sendFreshMenu, type MenuView } from "./telegram-menu";
import type { Env, NodeClass, NodeRow, OwnerRow, TelegramCallbackQuery, TelegramMessage, TelegramSessionRow } from "./models";
import { attachNodeToCluster, audit, clusterById, clusterByName, clusterMembershipCount, createClusterForOwner, createEnrollmentGrant, createNodeForOwner, nodeForOwnerById, nodeByName, ownerForTelegram, setTelegramSessionState, telegramSession } from "./store";
import { apiOrigin, filesOrigin, NODE_NAME, validNodeClass } from "./validation";

const NODE_ID = /^[0-9a-f-]{36}$/;

export async function handleMenuCallback(env: Env, callback: TelegramCallbackQuery): Promise<void> {
  const message = callback.message;
  if (!message) return answerCallback(env, callback.id, "❌ این دکمه در این گفتگو قابل استفاده نیست.");
  const owner = await ownerForTelegram(env, callback.from.id, callback.from.username);
  const session = await telegramSession(env, owner.id, message.chat.id);
  if (!session || session.menu_message_id !== message.message_id) {
    return answerCallback(env, callback.id, "⌛ این منو منقضی شده است. /start را ارسال کنید.");
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
  const session = await telegramSession(env, owner.id, message.chat.id);
  const text = message.text?.trim() ?? message.caption?.trim();
  if (text === "/start" || text === "/menu" || text === "/help") return showMessageHome(env, owner, message.chat.id);
  if (!session) return showMessageHome(env, owner, message.chat.id);

  try {
    if (session.mode === "cluster-create" && text) return createClusterFromInput(env, owner, message.chat.id, session, text);
    if (session.mode === "server-create" && text) return createServerFromInput(env, owner, message.chat.id, session, text);
    if (session.mode === "manifest-apply" && message.document) return applyManifestFromInput(env, owner, message.chat.id, session, message.document);
    await sendFreshMenu(env, owner, message.chat.id, pendingPrompt(session));
  } catch (reason) {
    await sendFreshMenu(env, owner, message.chat.id, { kind: "home", notice: displayError(reason) });
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
  if (extra !== undefined) throw new Error("دستور منو معتبر نیست.");
  if (action === "home") return showHome(env, owner, chatId);
  if (action === "help") return showPrompt(env, owner, chatId, "ℹ️ راهنما", "از دکمه‌های هر خوشه برای افزودن سرور داخلی یا خارجی استفاده کنید. هنگام درخواست، نام، نشانی سرور یا فایل تنظیمات را ارسال کنید.");
  if (action === "clusters") return showHome(env, owner, chatId);
  if (action === "nodes") return showView(env, owner, chatId, { kind: "nodes" });
  if (action === "cluster-add") return setPrompt(env, owner, chatId, "cluster-create", {}, "➕ خوشه جدید", "نام خوشه را با حروف کوچک، عدد و خط تیره ارسال کنید.");
  if (action === "cluster" && validId(value)) return showView(env, owner, chatId, { kind: "cluster", clusterId: value });
  if (action === "node" && validId(value)) return showNode(env, owner, chatId, session, value);
  if (action === "newd" && validId(value)) return setServerPrompt(env, owner, chatId, value, "domestic");
  if (action === "newf" && validId(value)) return setServerPrompt(env, owner, chatId, value, "foreign");
  if (action === "attach" && validId(value)) return showAttach(env, owner, chatId, value);
  if (action === "attach-node" && validId(value)) return attachExistingNode(env, owner, chatId, session, value);
  if (action === "install" && validId(value)) return issueInstall(env, owner, chatId, session, value);
  if (action === "manifest" && validId(value)) return setManifestPrompt(env, owner, chatId, session, value);
  throw new Error("این دکمه دیگر در دسترس نیست.");
}

async function showHome(env: Env, owner: OwnerRow, chatId: number): Promise<void> {
  const session = await setTelegramSessionState(env, owner.id, chatId, "idle", {});
  await presentMenu(env, owner, chatId, session, { kind: "home" });
}

async function showMessageHome(env: Env, owner: OwnerRow, chatId: number): Promise<void> {
  await setTelegramSessionState(env, owner.id, chatId, "idle", {});
  await sendFreshMenu(env, owner, chatId, { kind: "home" });
}

async function showView(env: Env, owner: OwnerRow, chatId: number, view: MenuView): Promise<void> {
  const active = await setTelegramSessionState(env, owner.id, chatId, "idle", viewPayload(view));
  await presentMenu(env, owner, chatId, active, view);
}

async function showNode(env: Env, owner: OwnerRow, chatId: number, session: TelegramSessionRow, nodeId: string): Promise<void> {
  const returnClusterId = payloadId(session, "returnClusterId", false);
  await showView(env, owner, chatId, { kind: "node", nodeId, returnClusterId });
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
  if (!await clusterById(env, owner.id, clusterId)) throw new Error("خوشه انتخاب‌شده دیگر وجود ندارد.");
  await setPrompt(env, owner, chatId, "server-create", { clusterId, nodeClass }, `➕ سرور ${nodeClassLabel(nodeClass)} جدید`, "`نام-سرور [نشانی]` را با حروف کوچک ارسال کنید.");
}

async function showAttach(env: Env, owner: OwnerRow, chatId: number, clusterId: string): Promise<void> {
  if (!await clusterById(env, owner.id, clusterId)) throw new Error("خوشه انتخاب‌شده دیگر وجود ندارد.");
  const session = await setTelegramSessionState(env, owner.id, chatId, "attach", { clusterId });
  await presentMenu(env, owner, chatId, session, { kind: "attach", clusterId });
}

async function attachExistingNode(env: Env, owner: OwnerRow, chatId: number, session: TelegramSessionRow, nodeId: string): Promise<void> {
  const clusterId = requiredPayloadId(session, "clusterId");
  const [cluster, node] = await Promise.all([clusterById(env, owner.id, clusterId), nodeForOwnerById(env, owner.id, nodeId)]);
  if (!cluster || !node) throw new Error("سرور یا خوشه انتخاب‌شده دیگر وجود ندارد.");
  await attachNodeToCluster(env, cluster.id, node.id, node.node_class);
  await audit(env, "cluster.node_attached", { node: node.name, role: node.node_class }, { ownerId: owner.id, nodeId: node.id, clusterId: cluster.id });
  const active = await setTelegramSessionState(env, owner.id, chatId, "idle", { returnClusterId: clusterId });
  await presentMenu(env, owner, chatId, active, { kind: "cluster", clusterId, notice: `✅ ${node.name} متصل شد.` });
}

async function issueInstall(env: Env, owner: OwnerRow, chatId: number, session: TelegramSessionRow, nodeId: string): Promise<void> {
  const node = await nodeForOwnerById(env, owner.id, nodeId);
  if (!node || await clusterMembershipCount(env, nodeId) < 1) throw new Error("سرور باید پیش از دریافت دستور نصب، عضو حداقل یک خوشه باشد.");
  const command = await installCommand(env, owner, node);
  await sendInstallCommand(env, chatId, node.name, command);
  const returnClusterId = payloadId(session, "returnClusterId", false);
  const active = await setTelegramSessionState(env, owner.id, chatId, "idle", { returnClusterId });
  await presentMenu(env, owner, chatId, active, { kind: "node", nodeId, returnClusterId, notice: "✅ دستور نصب یک‌بارمصرف ارسال شد." });
}

async function setManifestPrompt(env: Env, owner: OwnerRow, chatId: number, session: TelegramSessionRow, nodeId: string): Promise<void> {
  if (!await nodeForOwnerById(env, owner.id, nodeId)) throw new Error("سرور انتخاب‌شده دیگر وجود ندارد.");
  const returnClusterId = payloadId(session, "returnClusterId", false);
  await setPrompt(env, owner, chatId, "manifest-apply", { nodeId, returnClusterId }, "📝 اعمال وضعیت مطلوب", "فایل JSON نسخه ۳ با کدگذاری UTF-8 را اکنون پیوست کنید.");
}

async function createClusterFromInput(env: Env, owner: OwnerRow, chatId: number, _session: TelegramSessionRow, text: string): Promise<void> {
  const name = text.toLowerCase();
  if (!NODE_NAME.test(name)) throw new Error("نام خوشه فقط می‌تواند شامل حروف کوچک انگلیسی، عدد و خط تیره باشد.");
  if (await clusterByName(env, owner.id, name)) throw new Error(`خوشه ${name} از قبل وجود دارد.`);
  const cluster = await createClusterForOwner(env, owner.id, name);
  await audit(env, "cluster.created", { name }, { ownerId: owner.id, clusterId: cluster.id });
  await setTelegramSessionState(env, owner.id, chatId, "idle", { returnClusterId: cluster.id });
  await sendFreshMenu(env, owner, chatId, { kind: "cluster", clusterId: cluster.id, notice: `✅ خوشه ${name} ایجاد شد.` });
}

async function createServerFromInput(env: Env, owner: OwnerRow, chatId: number, session: TelegramSessionRow, text: string): Promise<void> {
  const [rawName, address, extra] = text.trim().split(/\s+/);
  const clusterId = requiredPayloadId(session, "clusterId");
  const nodeClass = payloadNodeClass(session);
  const name = rawName?.toLowerCase();
  if (!name || extra || !NODE_NAME.test(name)) throw new Error("`نام-سرور [نشانی]` را با نامی شامل حروف کوچک انگلیسی، عدد و خط تیره ارسال کنید.");
  if (!await clusterById(env, owner.id, clusterId)) throw new Error("خوشه انتخاب‌شده دیگر وجود ندارد.");
  if (await nodeByName(env, owner.id, name)) throw new Error(`${name} از قبل وجود دارد. آن را از منوی خوشه متصل کنید.`);
  const node = await createNodeForOwner(env, owner.id, name, nodeClass, address ?? null, nodeManifest(nodeClass));
  await attachNodeToCluster(env, clusterId, node.id, nodeClass);
  await audit(env, "cluster.node_attached", { node: name, role: nodeClass }, { ownerId: owner.id, nodeId: node.id, clusterId });
  await setTelegramSessionState(env, owner.id, chatId, "idle", { returnClusterId: clusterId });
  await sendFreshMenu(env, owner, chatId, { kind: "node", nodeId: node.id, returnClusterId: clusterId, notice: `✅ ${name} ایجاد و متصل شد.` });
}

async function applyManifestFromInput(env: Env, owner: OwnerRow, chatId: number, session: TelegramSessionRow, document: TelegramMessage["document"]): Promise<void> {
  if (!document) throw new Error("فایل JSON تنظیمات را پیوست کنید.");
  const nodeId = requiredPayloadId(session, "nodeId");
  const node = await nodeForOwnerById(env, owner.id, nodeId);
  if (!node) throw new Error("سرور انتخاب‌شده دیگر وجود ندارد.");
  const updated = await replaceNodeDocument(env, owner, node, await downloadTelegramDocument(env, document));
  const returnClusterId = payloadId(session, "returnClusterId", false);
  await setTelegramSessionState(env, owner.id, chatId, "idle", { returnClusterId });
  await sendFreshMenu(env, owner, chatId, { kind: "node", nodeId, returnClusterId, notice: `✅ نسخه ${updated.revision} اعمال شد.` });
}

export async function installCommand(env: Env, owner: OwnerRow, node: NodeRow): Promise<string> {
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
  if (session.mode === "cluster-create") return { kind: "prompt", title: "➕ خوشه جدید", detail: "نام خوشه را با حروف کوچک، عدد و خط تیره ارسال کنید." };
  if (session.mode === "server-create") return { kind: "prompt", title: "➕ سرور جدید", detail: "`نام-سرور [نشانی]` را با حروف کوچک ارسال کنید." };
  if (session.mode === "manifest-apply") return { kind: "prompt", title: "📝 اعمال وضعیت مطلوب", detail: "فایل JSON نسخه ۳ با کدگذاری UTF-8 را اکنون پیوست کنید." };
  return { kind: "home" };
}

function payloadId(session: TelegramSessionRow, key: string, required: boolean): string | undefined {
  const value = payload(session)[key];
  if (typeof value === "string" && validId(value)) return value;
  if (required) throw new Error("زمینه این منو منقضی شده است. منو را دوباره باز کنید.");
  return undefined;
}

function payloadNodeClass(session: TelegramSessionRow): NodeClass {
  const value = payload(session).nodeClass;
  if (typeof value !== "string" || !validNodeClass(value)) throw new Error("نوع سرور انتخاب‌شده منقضی شده است. منو را دوباره باز کنید.");
  return value;
}

function requiredPayloadId(session: TelegramSessionRow, key: string): string {
  const value = payloadId(session, key, false);
  if (!value) throw new Error("زمینه این منو منقضی شده است. منو را دوباره باز کنید.");
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
  const message = reason instanceof Error ? reason.message : "";
  if (message.startsWith("فایل JSON") || message.includes("manifest") || message.includes("document")) {
    return "❌ فایل وضعیت مطلوب معتبر نیست. ساختار و نسخه فایل را بررسی کنید.";
  }
  if (/[\u0600-\u06ff]/.test(message)) return `❌ ${message}`;
  return "❌ عملیات انجام نشد. دوباره تلاش کنید.";
}

function nodeClassLabel(nodeClass: NodeClass): string {
  return nodeClass === "domestic" ? "داخلی" : "خارجی";
}

function viewPayload(view: MenuView): Record<string, string> {
  if (view.kind === "cluster") return { returnClusterId: view.clusterId };
  if (view.kind === "node" && view.returnClusterId) return { returnClusterId: view.returnClusterId };
  return {};
}
