import { Show, createSignal } from "solid-js";
import { postOwner } from "../api/client";
import { OwnerDialog } from "./OwnerDialog";

export function ReleaseTokenDialog(props: { initData: string; close: () => void }) {
  const [token, setToken] = createSignal("");
  const [notice, setNotice] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  async function change(action: "rotate" | "revoke") {
    setBusy(true);
    setNotice("");
    try {
      const response = await postOwner("/release-token", props.initData, { action });
      const result = await response.json() as { token?: string };
      setToken(result.token ?? "");
      setNotice(action === "rotate" ? "Copy the new token before closing this dialog." : "Token revoked.");
    } catch (error) {
      setNotice(error instanceof Error ? error.message : "Token operation failed.");
    } finally {
      setBusy(false);
    }
  }
  async function copy() {
    try { await navigator.clipboard.writeText(token()); setNotice("Token copied."); }
    catch { setNotice("The clipboard was unavailable. Reopen Tunnel Owners and try again."); }
  }
  return <OwnerDialog class="node-popup lane-editor" title="release-token-title" close={props.close}>
    <header class="lane-editor__title">
      <h2 id="release-token-title">Release API token</h2>
      <button type="button" onClick={() => props.close()}>Close</button>
    </header>
    <p>This token expires after 30 days. Creating a new token revokes your previous one.</p>
    <div class="create-cluster__actions">
      <button type="button" disabled={busy()} onClick={() => void change("rotate")}>Create or rotate</button>
      <button type="button" disabled={busy()} onClick={() => void change("revoke")}>Revoke</button>
      <Show when={token()}><button type="button" onClick={() => void copy()}>Copy token</button></Show>
    </div>
    <Show when={notice()}><p role="status">{notice()}</p></Show>
  </OwnerDialog>;
}
