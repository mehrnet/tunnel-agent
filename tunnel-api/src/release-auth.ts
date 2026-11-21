import { error, json, parseJson } from "./http";
import type { Env, OwnerRow } from "./models";
import { equalSecret, now, parseGrant, randomToken, sha256 } from "./validation";
import { ownerFromTelegram } from "./web-auth";

const TOKEN_TTL_SECONDS = 30 * 24 * 60 * 60;

export function isReleaseOperator(env: Env, owner: Pick<OwnerRow, "telegram_user_id">) {
  return (env.RELEASE_OPERATOR_TELEGRAM_IDS ?? "").split(",").some(
    (value) => /^\d+$/.test(value.trim()) && value.trim() === String(owner.telegram_user_id),
  );
}

export async function handleReleaseToken(request: Request, env: Env): Promise<Response> {
  if (request.method !== "POST") return error("method not allowed", 405);
  if (request.headers.get("content-type")?.split(";", 1)[0] !== "application/json") {
    return error("expected application/json", 415);
  }
  const owner = await ownerFromTelegram(request, env);
  if (!owner) return error("unauthorized", 401);
  if (!isReleaseOperator(env, owner)) return error("release operator required", 403);
  const body = await parseJson(request);
  if (body?.action === "revoke") {
    await env.DB.prepare("DELETE FROM release_api_tokens WHERE owner_id=?").bind(owner.id).run();
    return json({ ok: true });
  }
  if (body?.action !== "rotate") return error("expected rotate or revoke");
  return createToken(env, owner.id);
}

async function createToken(env: Env, ownerId: string) {
  const id = crypto.randomUUID();
  const secret = randomToken();
  const expiresAt = now() + TOKEN_TTL_SECONDS;
  await env.DB.prepare(
    `INSERT INTO release_api_tokens (owner_id,token_id,secret_hash,expires_at) VALUES (?,?,?,?)
     ON CONFLICT(owner_id) DO UPDATE SET token_id=excluded.token_id,secret_hash=excluded.secret_hash,
       expires_at=excluded.expires_at`,
  ).bind(ownerId, id, await sha256(secret), expiresAt).run();
  return json({ token: `${id}.${secret}`, expires_at: expiresAt }, 201);
}

export async function releaseOperator(request: Request, env: Env): Promise<OwnerRow | null> {
  const header = request.headers.get("authorization");
  const token = parseGrant(header?.startsWith("Bearer ") ? header.slice(7) : null);
  if (!token) return null;
  const row = await env.DB.prepare(
    `SELECT o.id,o.telegram_user_id,t.secret_hash FROM release_api_tokens t
     JOIN owners o ON o.id=t.owner_id WHERE t.token_id=? AND t.expires_at>?`,
  ).bind(token.id, now()).first<OwnerRow & { secret_hash: string }>();
  if (!row || !isReleaseOperator(env, row) || !equalSecret(await sha256(token.secret), row.secret_hash)) return null;
  return { id: row.id, telegram_user_id: row.telegram_user_id };
}
