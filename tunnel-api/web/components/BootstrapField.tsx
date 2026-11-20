import { Show, createSignal } from "solid-js";
import { telegramContext } from "../platform/telegram";

export function BootstrapField(props: { nodeId: string; domesticGrant?: boolean }) {
  const [command, setCommand] = createSignal("");
  const [message, setMessage] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [publicKey, setPublicKey] = createSignal("");
  const label = () => props.domesticGrant ? "Identity grant" : "Bootstrap command";
  async function generate() {
    if (props.domesticGrant && !publicKey().trim()) return setMessage("Enter the Ed25519 public key from the domestic server.");
    setBusy(true); setMessage("");
    try {
      const endpoint = props.domesticGrant ? "domestic-grant" : "install";
      const response = await fetch(`/api/v1/web/nodes/${encodeURIComponent(props.nodeId)}/${endpoint}`, {
        method: "POST", headers: { "x-telegram-init-data": telegramContext().initData ?? "", "Content-Type": "application/json" },
        body: props.domesticGrant ? JSON.stringify({ public_key: publicKey().trim() }) : undefined,
      });
      const payload = await response.json() as { command?: string; grant?: string; error?: string };
      if (!response.ok) throw new Error(payload.error ?? `Request unavailable (HTTP ${response.status}).`);
      setCommand(payload.command ?? payload.grant ?? "");
    } catch (error) { setMessage(error instanceof Error ? error.message : "Request failed."); }
    finally { setBusy(false); }
  }
  async function copy() {
    try { await navigator.clipboard.writeText(command()); setMessage("Copied"); }
    catch { setMessage("Clipboard unavailable in this client."); }
  }
  return <div class="bootstrap-field"><strong>{label()}</strong>
    <Show when={props.domesticGrant}><label>Ed25519 SPKI public key (base64)
      <input value={publicKey()} onInput={(event) => setPublicKey(event.currentTarget.value)} maxlength={60} />
    </label></Show>
    <Show when={command()}><pre>{command()}</pre><button type="button" onClick={() => void copy()}>Copy {label().toLowerCase()}</button></Show>
    <button type="button" disabled={busy()} onClick={() => void generate()}>{busy() ? "Generating…" : `Generate ${label().toLowerCase()}`}</button><p>{message()}</p>
  </div>;
}
