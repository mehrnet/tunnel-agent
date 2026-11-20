interface StatePanelProps {
  state: "loading" | "empty" | "error" | "telegram";
  onRetry?: () => void;
  detail?: string;
}

const messages = {
  loading: ["Loading cluster", "Fetching nodes and connections."],
  empty: ["No clusters yet", "Add a cluster to get started."],
  error: ["Cluster data unavailable", "Check your connection and try again."],
  telegram: ["Open from Tunnel Owners", "Open this app using the button in TunnelOwnersBot."],
};

export function StatePanel(props: StatePanelProps) {
  const message = () => messages[props.state];
  return (
    <section class={`state-panel state-panel--${props.state}`} aria-live="polite">
      <span class="state-panel__mark" aria-hidden="true">{props.state === "error" ? "!" : props.state === "telegram" ? "TG" : "·"}</span>
      <div>
        <h2>{message()[0]}</h2>
        <p>{props.detail || message()[1]}</p>
      </div>
      {props.state === "error" && props.onRetry ? <button type="button" onClick={props.onRetry}>Retry</button> : null}
    </section>
  );
}
