import { createSignal, onCleanup, onMount } from "solid-js";
import { fetchCluster, fetchClusters } from "../api/client";
import type { ClusterResponse, ClusterSummary } from "../data/types";

type DataPhase = "ready" | "loading" | "empty" | "error" | "telegram";

export function createClusterData(initData: string | undefined, onClusterChange: () => void) {
  const [clusters, setClusters] = createSignal<ClusterSummary[]>([]);
  const [selectedId, setSelectedId] = createSignal("");
  const [cluster, setCluster] = createSignal<ClusterResponse>();
  const [phase, setPhase] = createSignal<DataPhase>(initData ? "loading" : "telegram");
  const [loadError, setLoadError] = createSignal("");
  let loadSequence = 0;
  let refreshing = false;

  function fail(reason: unknown) {
    setLoadError(reason instanceof Error ? reason.message : "Network request failed.");
    setPhase("error");
  }

  async function loadTelemetry(id: string) {
    if (!initData) return;
    const request = ++loadSequence;
    setPhase("loading");
    try {
      const next = await fetchCluster(id, initData);
      if (request !== loadSequence) return;
      setCluster(next); onClusterChange(); setPhase("ready");
    } catch (error) {
      if (request === loadSequence) fail(error);
    }
  }

  async function refreshSnapshot() {
    if (!initData || phase() !== "ready" || refreshing || document.hidden) return;
    const request = loadSequence;
    const id = selectedId();
    refreshing = true;
    try {
      const next = await fetchCluster(id, initData);
      if (request === loadSequence && id === selectedId() && phase() === "ready") setCluster(next);
    } catch {
      // Keep the last snapshot; observation timestamps will expire independently.
    } finally { refreshing = false; }
  }

  async function refresh() {
    if (!initData) return;
    setPhase("loading");
    try {
      const owned = await fetchClusters(initData);
      setClusters(owned);
      if (owned.length === 0) return setPhase("empty");
      const id = owned.find((item) => item.id === selectedId())?.id ?? owned[0].id;
      setSelectedId(id);
      await loadTelemetry(id);
    } catch (error) { fail(error); }
  }

  function chooseCluster(id: string) {
    setSelectedId(id);
    void loadTelemetry(id);
  }

  onMount(() => {
    if (!initData) return;
    void refresh();
    const poll = setInterval(() => void refreshSnapshot(), 15_000);
    onCleanup(() => { clearInterval(poll); ++loadSequence; });
  });
  return { clusters, setClusters, selectedId, cluster, phase, loadError, chooseCluster, refresh };
}
