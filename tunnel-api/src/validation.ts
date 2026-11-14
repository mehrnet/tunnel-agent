import type { Architecture, Env, NodeClass } from "./models";

export const BUNDLE_ROOT = "/opt/tunnel-agent/current";
export const NODE_NAME = /^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$/;
export const HEX_64 = /^[a-f0-9]{64}$/;
export const BUNDLE_VERSION = /^[0-9A-Za-z._+-]+$/;

export function now(): number {
  return Math.floor(Date.now() / 1000);
}

export function apiOrigin(env: Env): string {
  return env.API_ORIGIN.replace(/\/$/, "");
}

export function filesOrigin(env: Env): string {
  return env.FILES_ORIGIN.replace(/\/$/, "");
}

export function validNodeClass(value: string | undefined): value is NodeClass {
  return value === "domestic" || value === "foreign";
}

export function validArchitecture(value: string | undefined): value is Architecture {
  return value === "amd64" || value === "arm64";
}

export function randomToken(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export async function sha256(value: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export function equalSecret(actual: string | null, expected: string | undefined): boolean {
  if (!actual || !expected) return false;
  const length = Math.max(actual.length, expected.length);
  let difference = actual.length ^ expected.length;
  for (let index = 0; index < length; index += 1) {
    difference |= (actual.charCodeAt(index) || 0) ^ (expected.charCodeAt(index) || 0);
  }
  return difference === 0;
}

export function parseGrant(value: unknown): { id: string; secret: string } | null {
  if (typeof value !== "string") return null;
  const separator = value.indexOf(".");
  if (separator < 1) return null;
  const id = value.slice(0, separator);
  const secret = value.slice(separator + 1);
  if (!/^[0-9a-f-]{36}$/.test(id) || !HEX_64.test(secret)) return null;
  return { id, secret };
}

export function base64Utf8(value: string): string {
  const bytes = new TextEncoder().encode(value);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}
