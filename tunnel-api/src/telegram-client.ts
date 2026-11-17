import { MAX_MANIFEST_BYTES } from "./documents";
import type { Env, TelegramDocument } from "./models";

interface CallbackButton {
  text: string;
  callback_data: string;
}

interface WebAppButton {
  text: string;
  web_app: { url: string };
}

export type TelegramButton = CallbackButton | WebAppButton;

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
  if (!result || !Number.isInteger(result.message_id) || !Number.isInteger(result.date)) throw new Error("ارسال منو توسط تلگرام ناموفق بود.");
  return result;
}

async function sendText(env: Env, chatId: number, text: string): Promise<void> {
  if (text.length > 4096) throw new Error("متن از حد مجاز تلگرام طولانی‌تر است.");
  const result = await callTelegram<TelegramMessageRef>(env, "sendMessage", {
    chat_id: chatId,
    text,
    disable_web_page_preview: true,
  });
  if (!result) throw new Error("تلگرام پیام را نپذیرفت.");
}

export async function sendInstallCommand(env: Env, chatId: number, nodeName: string, command: string): Promise<void> {
  if (command.length <= 4096) return sendText(env, chatId, command);
  const token = env.TELEGRAM_BOT_TOKEN;
  if (!token) throw new Error("ارسال تلگرام پیکربندی نشده است.");
  const form = new FormData();
  form.set("chat_id", String(chatId));
  form.set("caption", "📄 دستور راه‌اندازی از حد مجاز پیام تلگرام طولانی‌تر است. فایل را باز کنید و تنها دستور داخل آن را کپی کنید.");
  form.set("document", new Blob([command], { type: "text/plain" }), `install-${nodeName}.txt`);
  const response = await fetch(`https://api.telegram.org/bot${token}/sendDocument`, { method: "POST", body: form });
  if (!response.ok) throw new Error("تلگرام فایل دستور نصب را نپذیرفت.");
  const payload = await response.json<TelegramResponse<TelegramMessageRef>>();
  if (!payload.ok) throw new Error("تلگرام فایل دستور نصب را نپذیرفت.");
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
  if (!document.file_name?.toLowerCase().endsWith(".json")) throw new Error("فایل پیوست باید با پسوند .json باشد.");
  const fileSize = document.file_size;
  if (typeof fileSize !== "number" || !Number.isSafeInteger(fileSize) || fileSize < 1 || fileSize > MAX_MANIFEST_BYTES) {
    throw new Error(`فایل پیوست باید بین ۱ تا ${MAX_MANIFEST_BYTES} بایت باشد.`);
  }
  const metadata = await callTelegram<{ file_path?: string }>(env, "getFile", { file_id: document.file_id });
  const path = metadata?.file_path;
  if (!path || !/^[A-Za-z0-9_./-]+$/.test(path) || path.includes("..")) throw new Error("تلگرام مسیر فایل نامعتبر برگرداند.");

  const response = await fetch(`${fileOrigin(env)}/${path}`);
  if (!response.ok) throw new Error("دریافت فایل پیوست ممکن نشد.");
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (bytes.byteLength > MAX_MANIFEST_BYTES) throw new Error(`فایل پیوست از ${MAX_MANIFEST_BYTES} بایت بزرگ‌تر است.`);
  try {
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: false }).decode(bytes);
  } catch {
    throw new Error("فایل پیوست متن UTF-8 نیست.");
  }
}

async function callTelegram<T>(env: Env, method: string, body: unknown): Promise<T | undefined> {
  const token = env.TELEGRAM_BOT_TOKEN;
  if (!token) throw new Error("ارسال تلگرام پیکربندی نشده است.");
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
  if (!token) throw new Error("ارسال تلگرام پیکربندی نشده است.");
  return `https://api.telegram.org/file/bot${token}`;
}
