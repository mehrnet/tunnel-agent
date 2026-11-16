import { MAX_MANIFEST_BYTES } from "./documents";
import type { Env, TelegramDocument } from "./models";

export interface TelegramButton {
  text: string;
  callback_data: string;
}

export interface TelegramMenu {
  text: string;
  keyboard: TelegramButton[][];
}

interface TelegramResponse<T> {
  ok?: boolean;
  result?: T;
}

interface TelegramMessageRef {
  message_id: number;
  date: number;
}

export async function sendMenu(env: Env, chatId: number, menu: TelegramMenu): Promise<TelegramMessageRef> {
  const result = await callTelegram<TelegramMessageRef>(env, "sendMessage", {
    chat_id: chatId,
    text: menu.text,
    disable_web_page_preview: true,
    reply_markup: { inline_keyboard: menu.keyboard },
  });
  if (!result || !Number.isInteger(result.message_id) || !Number.isInteger(result.date)) {
    throw new Error("Telegram did not return a menu message");
  }
  return result;
}

export async function sendText(env: Env, chatId: number, text: string): Promise<void> {
  if (text.length > 4096) throw new Error("Telegram text exceeds its message limit");
  const result = await callTelegram<TelegramMessageRef>(env, "sendMessage", {
    chat_id: chatId,
    text,
    disable_web_page_preview: true,
  });
  if (!result) throw new Error("Telegram did not accept the message");
}

export async function sendInstallCommand(env: Env, chatId: number, nodeName: string, command: string): Promise<void> {
  if (command.length <= 4096) return sendText(env, chatId, command);
  const token = env.TELEGRAM_BOT_TOKEN;
  if (!token) throw new Error("Telegram delivery is not configured");
  const form = new FormData();
  form.set("chat_id", String(chatId));
  form.set("caption", "The bootstrap command exceeded Telegram's message limit. Open this text file and copy its single command.");
  form.set("document", new Blob([command], { type: "text/plain" }), `install-${nodeName}.txt`);
  const response = await fetch(`https://api.telegram.org/bot${token}/sendDocument`, { method: "POST", body: form });
  if (!response.ok) throw new Error("Telegram did not accept the installation file");
  const payload = await response.json<TelegramResponse<TelegramMessageRef>>();
  if (!payload.ok) throw new Error("Telegram did not accept the installation file");
}

export async function editMenu(env: Env, chatId: number, messageId: number, menu: TelegramMenu): Promise<boolean> {
  const result = await callTelegram<boolean | TelegramMessageRef>(env, "editMessageText", {
    chat_id: chatId,
    message_id: messageId,
    text: menu.text,
    disable_web_page_preview: true,
    reply_markup: { inline_keyboard: menu.keyboard },
  });
  return result !== undefined;
}

export async function deleteMenu(env: Env, chatId: number, messageId: number): Promise<void> {
  await callTelegram<boolean>(env, "deleteMessage", { chat_id: chatId, message_id: messageId });
}

export async function answerCallback(env: Env, callbackId: string, text?: string): Promise<void> {
  await callTelegram<boolean>(env, "answerCallbackQuery", {
    callback_query_id: callbackId,
    ...(text ? { text, show_alert: true } : {}),
  });
}

export async function downloadTelegramDocument(env: Env, document: TelegramDocument): Promise<string> {
  if (!document.file_name?.toLowerCase().endsWith(".json")) throw new Error("attached manifest must be a .json file");
  const fileSize = document.file_size;
  if (typeof fileSize !== "number" || !Number.isSafeInteger(fileSize) || fileSize < 1 || fileSize > MAX_MANIFEST_BYTES) {
    throw new Error(`attached manifest must be between 1 and ${MAX_MANIFEST_BYTES} bytes`);
  }
  const metadata = await callTelegram<{ file_path?: string }>(env, "getFile", { file_id: document.file_id });
  const path = metadata?.file_path;
  if (!path || !/^[A-Za-z0-9_./-]+$/.test(path) || path.includes("..")) throw new Error("Telegram returned an invalid file path");

  const response = await fetch(`${fileOrigin(env)}/${path}`);
  if (!response.ok) throw new Error("cannot download the attached manifest");
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (bytes.byteLength > MAX_MANIFEST_BYTES) throw new Error(`attached manifest exceeds ${MAX_MANIFEST_BYTES} bytes`);
  try {
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: false }).decode(bytes);
  } catch {
    throw new Error("attached manifest is not UTF-8 text");
  }
}

async function callTelegram<T>(env: Env, method: string, body: unknown): Promise<T | undefined> {
  const token = env.TELEGRAM_BOT_TOKEN;
  if (!token) throw new Error("Telegram delivery is not configured");
  const response = await fetch(`https://api.telegram.org/bot${token}/${method}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) return undefined;
  const payload = await response.json<TelegramResponse<T>>();
  return payload.ok ? payload.result : undefined;
}

function fileOrigin(env: Env): string {
  const token = env.TELEGRAM_BOT_TOKEN;
  if (!token) throw new Error("Telegram delivery is not configured");
  return `https://api.telegram.org/file/bot${token}`;
}
