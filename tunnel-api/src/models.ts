export interface Env {
  DB: D1Database;
  API_ORIGIN: string;
  FILES_ORIGIN: string;
  DOMESTIC_BOOTSTRAP_URL?: string;
  TELEGRAM_BOT_USERNAME: string;
  ENROLLMENT_TTL_SECONDS: string;
  TELEGRAM_BOT_TOKEN?: string;
  TELEGRAM_WEBHOOK_SECRET?: string;
  BUNDLE_PUBLISH_TOKEN?: string;
  CAPSULE_SIGNING_PRIVATE_KEY?: string;
  CLUSTER_CONTROL_KEY_ENCRYPTION_KEY?: string;
  DOMESTIC_CAPSULE_TTL_SECONDS?: string;
  DOMESTIC_CAPSULE_BUNDLE_VERSION?: string;
}

export type NodeClass = "domestic" | "foreign";
export type Architecture = "amd64" | "arm64";

export interface OwnerRow {
  id: string;
  telegram_user_id: string;
}

export interface ClusterRow {
  id: string;
  name: string;
}

export interface ClusterControlKeyRow {
  ssh_public_key: string;
}

export interface RelayRow {
  address: string;
}

export interface NodeRow {
  id: string;
  name: string;
  node_class: NodeClass;
  address: string | null;
  status: "pending" | "enrolled" | "revoked";
}

export interface GrantRow extends NodeRow {
  grant_id: string;
  secret_hash: string;
  expires_at: number;
  consumed_at: number | null;
}

export interface BundleRow {
  version: string;
  architecture: Architecture;
  sha256: string;
  inventory_sha256: string | null;
  public_url: string;
  domestic_url: string | null;
  domestic_urls: string | null;
}

export interface DocumentRow {
  revision: number;
  manifest_json: string;
  updated_at: number;
}

export interface RelayDocumentRow extends DocumentRow {
  cluster_id: string;
  node_id: string;
}

export interface BundleActivationEntry {
  architecture: Architecture;
  sha256: string;
  inventory_sha256: string;
  public_url: string;
  domestic_urls: string[];
}

export interface BundleActivation {
  version: string;
  bundles: BundleActivationEntry[];
}

export interface TelegramUpdate {
  update_id: number;
  message?: TelegramMessage;
  callback_query?: TelegramCallbackQuery;
}

export interface TelegramUser {
  id: number;
  username?: string;
}

export interface TelegramChat {
  id: number;
}

export interface TelegramMessage {
  message_id: number;
  date: number;
  chat: TelegramChat;
  from?: TelegramUser;
  text?: string;
  caption?: string;
  document?: TelegramDocument;
}

export interface TelegramCallbackQuery {
  id: string;
  from: TelegramUser;
  data?: string;
  message?: TelegramMessage;
}

export interface TelegramDocument {
  file_id: string;
  file_name?: string;
  file_size?: number;
}

export interface TelegramSessionRow {
  owner_id: string;
  chat_id: string;
  mode: string;
  payload_json: string;
  menu_message_id: number | null;
  menu_created_at: number | null;
  updated_at: number;
}

export interface AuditReferences {
  ownerId?: string;
  nodeId?: string;
  clusterId?: string;
}
