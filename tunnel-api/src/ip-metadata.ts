export interface IpMetadata {
  country_code: string | null;
  region: string | null;
  city: string | null;
  prefix: string | null;
  registry: string | null;
  origins: Array<{ asn: string; name: string | null; organization: string | null }>;
  checked_at: number;
}

function publicIpv6(address: string): string | null {
  const host = address.startsWith("[") && address.endsWith("]") ? address : `[${address}]`;
  if (!/^\[[0-9a-f:.]+\]$/i.test(host)) return null;
  try {
    const ip = new URL(`http://${host}`).hostname.slice(1, -1);
    return /^[23][0-9a-f]{3}:/.test(ip) && !ip.startsWith("2001:db8:") ? ip : null;
  } catch { return null; }
}

export function publicIp(address: string): string | null {
  if (address.includes(":")) return publicIpv6(address);
  if (!/^(?:0|[1-9]\d{0,2})(?:\.(?:0|[1-9]\d{0,2})){3}$/.test(address)) return null;
  const octets = address.split(".").map(Number);
  const [a, b, c] = octets;
  if (octets.some((part) => part > 255) || [0, 10, 127].includes(a) || a >= 224
    || (a === 100 && b >= 64 && b <= 127) || (a === 169 && b === 254) || (a === 172 && b >= 16 && b <= 31)
    || (a === 192 && (b === 168 || (b === 0 && [0, 2].includes(c))))
    || (a === 198 && ([18, 19].includes(b) || (b === 51 && c === 100)))
    || (a === 203 && b === 0 && c === 113)) return null;
  return address;
}

function object(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function field(value: unknown): string | null {
  return typeof value === "string" ? value.trim().slice(0, 160) || null : null;
}

function country(value: unknown): string | null {
  return typeof value === "string" && /^[A-Z]{2}$/.test(value) && !["EU", "ZZ"].includes(value) ? value : null;
}

function asn(value: unknown): string | null {
  if (typeof value === "string" && /^AS\d{1,10}$/.test(value) && Number(value.slice(2)) <= 4294967295) return value;
  return null;
}

function parseOrigins(network: Record<string, unknown>): IpMetadata["origins"] {
  const origins = new Map<string, IpMetadata["origins"][number]>();
  for (const value of Array.isArray(network.origins) ? network.origins.slice(0, 32) : []) {
    const origin = object(value);
    const id = asn(origin.asn);
    if (id) origins.set(id, { asn: id, name: field(origin.name), organization: field(origin.organization) });
  }
  const ids = Array.isArray(network.asns) ? network.asns.slice(0, 32) : [network.asn];
  for (const value of ids) {
    const id = asn(value);
    if (id && !origins.has(id)) origins.set(id, { asn: id, name: null, organization: null });
  }
  return [...origins.values()];
}

export function parseIpMetadata(value: unknown, ip: string, checkedAt: number): IpMetadata | null {
  const reply = object(value);
  if (typeof reply.ip !== "string" || publicIp(reply.ip) !== ip) return null;
  const network = object(reply.network);
  const allocation = object(reply.allocation);
  const location = object(reply.location);
  const result = {
    country_code: country(location.country_code) ?? country(allocation.country_code),
    region: field(location.region), city: field(location.city), prefix: field(network.cidr),
    registry: field(reply.registry) ?? field(allocation.registry),
    origins: parseOrigins(network),
    checked_at: checkedAt,
  };
  return result.country_code || result.city || result.region || result.prefix || result.registry || result.origins.length ? result : null;
}
