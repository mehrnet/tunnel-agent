import { error, json } from "./http";
import { handleMenuCallback, handleMenuMessage } from "./telegram-flow";
import type { Env, TelegramUpdate } from "./models";
import { equalSecret, now } from "./validation";

export async function handleTelegram(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  if (!equalSecret(request.headers.get("X-Telegram-Bot-Api-Secret-Token"), env.TELEGRAM_WEBHOOK_SECRET)) {
    return error("unauthorized", 401);
  }
  const update = await parseUpdate(request);
  if (!update) return error("invalid Telegram update");
  const recorded = await recordUpdate(env, update.update_id);
  if (!recorded) return json({ ok: true });
  if (update.callback_query) ctx.waitUntil(handleMenuCallback(env, update.callback_query));
  if (update.message) ctx.waitUntil(handleMenuMessage(env, update.message));
  return json({ ok: true });
}

async function parseUpdate(request: Request): Promise<TelegramUpdate | null> {
  try {
    const update = await request.json<TelegramUpdate>();
    return Number.isInteger(update.update_id) ? update : null;
  } catch {
    return null;
  }
}

async function recordUpdate(env: Env, updateId: number): Promise<boolean> {
  const result = await env.DB.prepare(
    "INSERT OR IGNORE INTO telegram_updates (update_id, received_at) VALUES (?, ?)",
  ).bind(updateId, now()).run();
  return result.meta.changes === 1;
}
