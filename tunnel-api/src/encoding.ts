const BASE62 = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

export function base64Url(bytes: ArrayBuffer | Uint8Array): string {
  const source = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let binary = "";
  for (const byte of source) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

export function base64Standard(value: string): Uint8Array | null {
  try {
    const binary = atob(value);
    return Uint8Array.from(binary, (character) => character.charCodeAt(0));
  } catch {
    return null;
  }
}

export function base62Capsule(value: string): string {
  const bytes = new TextEncoder().encode(value);
  if (bytes.length === 0 || bytes.length >= 62 ** 5) throw new Error("capsule payload is too large");
  let output = `ta1${base62Block(BigInt(bytes.length), 5)}`;
  for (let index = 0; index < bytes.length; index += 5) {
    let block = 0n;
    for (let offset = 0; offset < 5; offset += 1) block = (block << 8n) | BigInt(bytes[index + offset] ?? 0);
    output += base62Block(block, 7);
  }
  return output;
}

function base62Block(value: bigint, width: number): string {
  let output = "";
  for (let index = 0; index < width; index += 1) {
    output = BASE62[Number(value % 62n)] + output;
    value /= 62n;
  }
  return output;
}
