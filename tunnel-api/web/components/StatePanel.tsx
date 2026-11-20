import { Show } from "solid-js";
import { WorkbenchIcon } from "./WorkbenchIcon";

interface StatePanelProps {
  state: "loading" | "empty" | "error" | "telegram";
  onRetry?: () => void;
  detail?: string;
}

const messages = {
  loading: ["Loading cluster", "Retrieving server and lane information."],
  empty: ["No clusters yet", "Use the + in the cluster bar to create your first cluster."],
  error: ["Cluster data unavailable", "Check your connection and try again."],
  telegram: ["Open from Tunnel Owners", "Open this app using the button in TunnelOwnersBot."],
};

export function StatePanel(props: StatePanelProps) {
  const message = () => messages[props.state];
  return (
    <section class={`state-panel state-panel--${props.state}`} aria-labelledby="state-title">
      <span class="state-panel__mark"><WorkbenchIcon name={props.state === "error" ? "warning" : props.state === "telegram" ? "telegram" : props.state === "loading" ? "refresh" : "map"} /></span>
      <div role={props.state === "error" ? "alert" : "status"}>
        <p class="section-label">Network operations</p>
        <h2 id="state-title">{message()[0]}</h2>
        <p>{props.detail || message()[1]}</p>
      </div>
      <Show when={props.state === "error" && props.onRetry}><button type="button" onClick={() => props.onRetry?.()}><WorkbenchIcon name="refresh" />Retry</button></Show>
    </section>
  );
}
