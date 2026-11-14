import { base64Standard, base64Url } from "./encoding";
import type { Env } from "./models";

export async function signApiPayload(env: Env, payload: string): Promise<string> {
  const privateKey = base64Standard(env.CAPSULE_SIGNING_PRIVATE_KEY ?? "");
  if (!privateKey) throw new Error("API document signing is not configured");
  const key = await crypto.subtle.importKey("pkcs8", privateKey, { name: "Ed25519" }, false, ["sign"]);
  const signature = await crypto.subtle.sign("Ed25519", key, new TextEncoder().encode(payload));
  return base64Url(signature);
}
