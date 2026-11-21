import { Index, Show, createMemo, createSignal } from "solid-js";
import type { ClusterNode, ClusterResponse, Selection, TunnelLane } from "../data/types";
import { laneHealth, listenerLabel, nodeStatus, routeKind } from "../lib/workbench-status";
import { formatMoment } from "../lib/format";
import { WorkbenchIcon } from "./WorkbenchIcon";

interface InventoryItem {
  kind: "node" | "lane";
  id: string;
  title: string;
  subtitle: string;
  status: string;
  terms: string;
}

function nodeItem(node: ClusterNode): InventoryItem {
  const role = node.node_class === "foreign" ? "Foreign" : "Domestic";
  const status = nodeStatus(node);
  return {
    kind: "node", id: node.id, title: node.name, status,
    subtitle: [role, node.address].filter(Boolean).join(" / "),
    terms: [node.name, node.id, role, node.address, status].join(" ").toLowerCase(),
  };
}

function laneItem(lane: TunnelLane, names: Map<string, string>): InventoryItem {
  const source = names.get(lane.source_node_id) ?? "Unknown server";
  const peer = names.get(lane.peer_node_id) ?? "Unknown server";
  const kind = routeKind(lane);
  const status = laneHealth(lane);
  return {
    kind: "lane", id: lane.id, title: `${source} → ${peer}`, status, subtitle: [lane.transport, listenerLabel(lane), kind].filter(Boolean).join(" / "),
    terms: [source, peer, lane.id, kind, lane.transport, lane.direction, lane.local_listener, status].join(" ").toLowerCase(),
  };
}

function InventoryGroup(props: {
  title: string;
  items: InventoryItem[];
  selection: Selection;
  onSelect: (selection: Selection) => void;
}) {
  return <Show when={props.items.length > 0}>
    <section class="inventory-group" aria-label={props.title}>
      <h3 class="section-label">{props.title}<span>{props.items.length}</span></h3>
      <ul>
        <Index each={props.items}>{(item) =>
          <li><button class="inventory-row" type="button" aria-pressed={props.selection?.kind === item().kind && props.selection.id === item().id}
            title={`${item().title} / ${item().subtitle} / ${item().status}`} onClick={() => props.onSelect({ kind: item().kind, id: item().id })}>
            <span class="inventory-row__icon"><WorkbenchIcon name={item().kind === "node" ? "server" : "lanes"} /></span>
            <span class="inventory-row__text"><strong>{item().title}</strong><small>{item().subtitle}</small></span>
            <span class="inventory-row__status" data-status={item().status}>{item().status}</span>
            <WorkbenchIcon name="chevron" />
          </button></li>
        }</Index>
      </ul>
    </section>
  </Show>;
}

export function InventoryPanel(props: {
  cluster: ClusterResponse;
  selection: Selection;
  onSelect: (selection: Selection) => void;
}) {
  const [query, setQuery] = createSignal("");
  const terms = createMemo(() => query().trim().toLowerCase().split(/\s+/).filter(Boolean));
  const matches = (item: InventoryItem) => terms().every((term) => item.terms.includes(term));
  const nodes = createMemo(() => props.cluster.nodes.map(nodeItem).filter(matches));
  const lanes = createMemo(() => {
    const names = new Map(props.cluster.nodes.map((node) => [node.id, node.name]));
    return props.cluster.lanes.map((lane) => laneItem(lane, names)).filter(matches);
  });
  const active = () => props.cluster.nodes.filter((node) => nodeStatus(node) === "Active").length;
  const degraded = () => props.cluster.lanes.filter((lane) => laneHealth(lane) === "Degraded").length;
  return <section id="inventory-panel" class="inventory-panel" aria-labelledby="inventory-title">
    <header class="workspace-panel-heading">
      <div><p class="section-label">{props.cluster.cluster.name}</p><h2 id="inventory-title">Inventory</h2></div>
      <p class="inventory-panel__summary">
        <span>{active()}/{props.cluster.nodes.length} servers active</span>
        <span classList={{ "has-degraded": degraded() > 0 }}>{degraded()}/{props.cluster.lanes.length} lanes degraded</span>
        <span>Snapshot {formatMoment(props.cluster.generated_at)}</span>
      </p>
    </header>
    <div class="inventory-panel__search">
      <WorkbenchIcon name="search" />
      <input type="search" aria-label="Search servers and lanes" placeholder="Find a server or lane…" value={query()} onInput={(event) => setQuery(event.currentTarget.value)} />
    </div>
    <div class="inventory-panel__list">
      <Show when={nodes().length || lanes().length} fallback={
        <div class="inventory-empty" role="status"><strong>{terms().length ? "No matches" : "No assets yet"}</strong>
          <p>{terms().length ? "Try a name, address, transport, or status." : "New servers and lanes will appear here."}</p>
          <Show when={terms().length}><button type="button" onClick={() => setQuery("")}>Clear search</button></Show>
        </div>
      }>
        <InventoryGroup title="Servers" items={nodes()} selection={props.selection} onSelect={props.onSelect} />
        <InventoryGroup title="Lanes" items={lanes()} selection={props.selection} onSelect={props.onSelect} />
      </Show>
    </div>
  </section>;
}
