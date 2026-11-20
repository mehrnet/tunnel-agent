import { base64Standard } from "./encoding";

export interface TransportKeys {
  hpnssh: string | null;
  rathole: string | null;
  kernel_ingress?: boolean;
}

function hpnKey(value: string): boolean {
  if (value.length !== 68) return false;
  const raw = base64Standard(value);
  const prefix = [0, 0, 0, 11, ...new TextEncoder().encode("ssh-ed25519"), 0, 0, 0, 32];
  return raw?.length === prefix.length + 32 && prefix.every((byte, index) => raw[index] === byte);
}

function noiseKey(value: string): boolean {
  if (value.length !== 44) return false;
  const raw = base64Standard(value);
  return raw?.length === 32 && raw.some((byte) => byte !== 0);
}

export function parseTransportKeys(value: unknown): TransportKeys | null {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return null;
  const { hpnssh, rathole, kernel_ingress } = value as Record<string, unknown>;
  if (hpnssh !== null && (typeof hpnssh !== "string" || !hpnKey(hpnssh))) return null;
  if (rathole !== null && (typeof rathole !== "string" || !noiseKey(rathole))) return null;
  if (kernel_ingress !== undefined && typeof kernel_ingress !== "boolean") return null;
  return { hpnssh, rathole, kernel_ingress };
}
