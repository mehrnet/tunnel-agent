import type { Env, OwnerRow } from "./models";
import { ownerForTelegram } from "./store";

const MAX_AGE_SECONDS = 60 * 60;
const sessions = new WeakMap<Request, string>();

export function webSessionCookie(request: Request): string | undefined { return sessions.get(request); }

export async function ownerFromTelegram(request: Request, env: Env): Promise<OwnerRow | null> {
  const initData = request.headers.get("x-telegram-init-data");
  const token = env.TELEGRAM_BOT_TOKEN;
  if (!token) return null;
  const session = request.headers.get("cookie")?.match(/(?:^|;\s*)__Host-tunnel-session=([^;]+)/)?.[1];
  const restored = session ? await restoreSession(session, token) : null;
  if (restored) return ownerForTelegram(env, restored.id);
  const user = initData ? await verifyInitData(initData, token) : null;
  if (user) {
    const payload = `${user.id}.${Math.floor(Date.now() / 1000) + 43200}`;
    const signature = hex(await hmac(`tunnel-web-session:${token}`, payload));
    sessions.set(request, `__Host-tunnel-session=${payload}.${signature}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=43200`);
  }
  return user ? ownerForTelegram(env, user.id, user.username) : null;
}

async function restoreSession(raw: string, token: string): Promise<{ id: number } | null> {
  const [id, expiry, signature, extra] = raw.split(".");
  if (extra || !signature || !Number.isSafeInteger(Number(id)) || Number(id) < 1 || !Number.isSafeInteger(Number(expiry))) return null;
  const now = Math.floor(Date.now() / 1000);
  if (Number(expiry) <= now || Number(expiry) > now + 43200) return null;
  const expected = hex(await hmac(`tunnel-web-session:${token}`, `${id}.${expiry}`));
  return equalHex(expected, signature) ? { id: Number(id) } : null;
}

async function verifyInitData(initData: string, token: string): Promise<{ id: number; username?: string } | null> {
  const params = new URLSearchParams(initData);
  const hash = params.get("hash");
  const authDate = Number(params.get("auth_date"));
  const rawUser = params.get("user");
  if (!hash || !/^[0-9a-f]{64}$/i.test(hash) || !Number.isSafeInteger(authDate) || !rawUser) return null;
  if (Math.abs(Math.floor(Date.now() / 1000) - authDate) > MAX_AGE_SECONDS) return null;
  const user = parseUser(rawUser);
  if (!user) return null;
  const entries = [...params.entries()].filter(([key]) => key !== "hash").sort(([left], [right]) => left.localeCompare(right));
  if (entries.some(([key], index) => index > 0 && key === entries[index - 1][0])) return null;
  const checkString = entries.map(([key, value]) => `${key}=${value}`).join("\n");
  const secret = await hmac("WebAppData", token);
  const expected = hex(await hmac(secret, checkString));
  return equalHex(expected, hash) ? user : null;
}

function parseUser(raw: string): { id: number; username?: string } | null {
  try {
    const value: unknown = JSON.parse(raw);
    if (value === null || typeof value !== "object" || Array.isArray(value)) return null;
    const user = value as Record<string, unknown>;
    if (typeof user.id !== "number" || !Number.isSafeInteger(user.id) || user.id < 1) return null;
    return typeof user.username === "string" ? { id: user.id, username: user.username } : { id: user.id };
  } catch {
    return null;
  }
}

async function hmac(key: string | ArrayBuffer, value: string): Promise<ArrayBuffer> {
  const encoder = new TextEncoder();
  const imported = await crypto.subtle.importKey("raw", typeof key === "string" ? encoder.encode(key) : key, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return crypto.subtle.sign("HMAC", imported, encoder.encode(value));
}

function hex(value: ArrayBuffer): string {
  return [...new Uint8Array(value)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function equalHex(left: string, right: string): boolean {
  if (left.length !== right.length) return false;
  let difference = 0;
  for (let index = 0; index < left.length; index += 1) difference |= left.charCodeAt(index) ^ right.charCodeAt(index);
  return difference === 0;
}
