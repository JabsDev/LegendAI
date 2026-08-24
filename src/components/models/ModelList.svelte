<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import { invoke } from "@tauri-apps/api/core";
  import ModelDownload from "./ModelDownload.svelte";
  import { showError } from "../common/ErrorToast.svelte";
  import { errMsg } from "../../lib/errors";
  import { t } from "../../lib/t";

  type ModelKind = "stt" | "translation";
  type Backend = "whisper" | "llama" | "ort" | "parakeet" | "canary" | "nemotron";
  type CacheStatus = "downloading" | "downloaded" | "error";

  interface ModelInfo {
    id: string;
    kind: ModelKind;
    name: string;
    repo_id: string;
    file: string;
    backend: Backend;
    quantization: string;
    size_mb: number;
    min_ram_gb: number;
    quality: number;
    speed: number;
    threads_supported: boolean;
  }

  interface Catalog {
    version: number;
    models: ModelInfo[];
  }

  interface ModelCacheStatus {
    model_id: string;
    status: CacheStatus | null;
  }

  interface ActiveModels {
    stt: string;
    translation: string;
  }

  const STARS = [1, 2, 3, 4, 5];

  const BACKEND_LABEL = $derived<Record<Backend, string>>({
    whisper: t("models.backend.whisper"),
    llama: t("models.backend.llama"),
    ort: t("models.backend.ort"),
    parakeet: "Parakeet",
    canary: "Canary",
    nemotron: "Nemotron",
  });

  const UNSUPPORTED_STT = new Set<Backend>(["canary", "nemotron"]);

  function isSupported(m: ModelInfo): boolean {
    if (m.kind === "stt" && UNSUPPORTED_STT.has(m.backend)) return false;
    return true;
  }

  const STATUS_LABEL = $derived<Record<CacheStatus, string>>({
    downloaded: t("models.status.downloaded"),
    downloading: t("models.status.downloading"),
    error: t("models.status.error"),
  });

  let models: ModelInfo[] = $state([]);
  let statuses: Map<string, CacheStatus | null> = $state(new Map());
  let loadError: string | null = $state(null);
  let activeKind: ModelKind = $state("stt");
  let active: ActiveModels = $state({ stt: "", translation: "" });
  let activeWarning: string | null = $state(null);

  const filtered = $derived(models.filter((m) => m.kind === activeKind));

  function baseId(id: string): string {
    return id.replace(/-q\d.*$/, "").replace(/-q4.*$/, "");
  }

  function groupLabel(m: ModelInfo): string {
    return m.name.replace(/\s*\(.*\)\s*$/, "").trim();
  }

  const grouped = $derived.by(() => {
    const map = new SvelteMap<string, ModelInfo[]>();
    for (const m of filtered) {
      const k = baseId(m.id);
      if (!map.has(k)) map.set(k, []);
      map.get(k)!.push(m);
    }
    // dentro do grupo: melhor qualidade primeiro
    for (const [, arr] of map) arr.sort((a, b) => b.quality - a.quality || b.speed - a.speed);
    return Array.from(map.entries()).map(([k, arr]) => ({
      key: k,
      label: groupLabel(arr[0]),
      backend: arr[0].backend,
      models: arr,
    }));
  });

  let expanded = new SvelteSet<string>();

  function toggleGroup(k: string) {
    if (expanded.has(k)) expanded.delete(k);
    else expanded.add(k);
  }

  // expande automaticamente o grupo que contém o modelo ativo
  $effect(() => {
    const cur = active[activeKind];
    if (!cur) return;
    const g = baseId(cur);
    if (!expanded.has(g)) expanded.add(g);
  });

  onMount(() => {
    void load();
    void loadActive();
  });

  async function load(): Promise<void> {
    try {
      const [catalog, cacheStatus] = await Promise.all([
        invoke<Catalog>("list_catalog"),
        invoke<ModelCacheStatus[]>("list_cache_status"),
      ]);
      models = catalog.models;
      statuses = new Map(cacheStatus.map((s) => [s.model_id, s.status]));
    } catch (e) {
      loadError = errMsg(e, "models.errLoad");
      showError(e);
    }
  }

  async function loadActive(): Promise<void> {
    try {
      active = await invoke<ActiveModels>("get_active_models");
    } catch (e) {
      loadError = errMsg(e, "models.errLoadActive");
      showError(e);
    }
  }

  async function setActive(m: ModelInfo): Promise<void> {
    activeWarning = null;
    try {
      const warning = await invoke<string | null>("set_active_model", {
        kind: m.kind,
        id: m.id,
      });
      active = { ...active, [m.kind]: m.id };
      if (warning) activeWarning = warning;
    } catch (e) {
      activeWarning = errMsg(e, "models.errSetActive");
      showError(e);
    }
  }

  async function refreshStatuses(): Promise<void> {
    try {
      const cacheStatus = await invoke<ModelCacheStatus[]>("list_cache_status");
      statuses = new Map(cacheStatus.map((s) => [s.model_id, s.status]));
    } catch (e) {
      loadError = errMsg(e, "models.errRefresh");
      showError(e);
    }
  }

  function statusLabel(id: string): string {
    const s = statuses.get(id);
    return s === undefined || s === null ? t("models.status.none") : STATUS_LABEL[s];
  }

  function statusClass(id: string): string {
    return statuses.get(id) === null ? "none" : `status-${statuses.get(id)}`;
  }

  function formatSize(mb: number): string {
    return mb >= 1024
      ? t("models.sizeGb", { value: (mb / 1024).toFixed(1) })
      : t("models.sizeMb", { value: mb });
  }
</script>

<div class="model-manager">
  <div class="tabs" role="tablist" aria-label={t("models.tabsAria")}>
    <button
      type="button"
      class="tab"
      class:active={activeKind === "stt"}
      role="tab"
      aria-selected={activeKind === "stt"}
      onclick={() => (activeKind = "stt")}
    >
      {t("models.tabStt")}
    </button>
    <button
      type="button"
      class="tab"
      class:active={activeKind === "translation"}
      role="tab"
      aria-selected={activeKind === "translation"}
      onclick={() => (activeKind = "translation")}
    >
      {t("models.tabTranslation")}
    </button>
  </div>

  {#if loadError}
    <p class="error" role="alert">{loadError}</p>
  {:else if grouped.length === 0}
    <p class="empty">{t("models.empty")}</p>
  {:else}
    <div class="groups">
      {#each grouped as g (g.key)}
        {@const isOpen = expanded.has(g.key)}
        {@const isActiveGroup = g.models.some((m) => active[m.kind] === m.id)}
        <div class="group" class:open={isOpen} class:active-group={isActiveGroup}>
          <button
            type="button"
            class="group-head"
            onclick={() => toggleGroup(g.key)}
            aria-expanded={isOpen}
          >
            <span class="group-title">
              {g.label}
              <span class="badge backend-{g.backend}">{BACKEND_LABEL[g.backend as Backend]}</span>
              {#if isActiveGroup}<span class="badge status-downloaded">{t("models.active")}</span
                >{/if}
            </span>
            <span class="group-meta"
              >{g.models.length}
              {g.models.length === 1 ? "variante" : "variantes"} · {isOpen ? "▾" : "▸"}</span
            >
          </button>
          {#if isOpen}
            <table class="models">
              <thead>
                <tr>
                  <th>{t("models.colModel")}</th>
                  <th>{t("models.colSize")}</th>
                  <th>{t("models.colQuality")}</th>
                  <th>{t("models.colStatus")}</th>
                  <th>{t("models.colActions")}</th>
                </tr>
              </thead>
              <tbody>
                {#each g.models as m (m.id)}
                  {@const supported = isSupported(m)}
                  <tr class:active-row={active[m.kind] === m.id} class:unsupported={!supported}>
                    <td>
                      <span class="name">{m.name}</span>
                      <span class="quant">{m.quantization}</span>
                      {#if !supported}<span class="badge status-error" style="margin-left: 6px;"
                          >Em breve</span
                        >{/if}
                    </td>
                    <td>{formatSize(m.size_mb)}</td>
                    <td
                      class="quality"
                      title={t("models.qualityTitle", { quality: m.quality, speed: m.speed })}
                    >
                      {#each STARS as n (n)}
                        <span class="star" class:on={n <= m.quality}>★</span>
                      {/each}
                    </td>
                    <td><span class={`badge ${statusClass(m.id)}`}>{statusLabel(m.id)}</span></td>
                    <td>
                      <div class="actions">
                        <button
                          type="button"
                          class="activate"
                          class:active={active[m.kind] === m.id}
                          disabled={!supported}
                          title={supported
                            ? t(
                                activeKind === "stt"
                                  ? "models.useFor.stt"
                                  : "models.useFor.translation",
                              )
                            : "Backend ainda não implementado — selecione um Whisper para transcrever"}
                          onclick={() => supported && setActive(m)}
                        >
                          {active[m.kind] === m.id ? t("models.active") : t("models.activate")}
                        </button>
                        <ModelDownload
                          model={m}
                          status={statuses.get(m.id) ?? null}
                          onStatusChange={refreshStatuses}
                        />
                      </div>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
  {#if activeWarning}
    <p class="warn" role="alert">{activeWarning}</p>
  {/if}
</div>

<style>
  .model-manager {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 860px;
  }

  .tabs {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid color-mix(in srgb, currentColor 20%, transparent);
  }

  .tab {
    padding: 6px 14px;
    border: none;
    border-bottom: 2px solid transparent;
    background: none;
    font: inherit;
    cursor: pointer;
    color: inherit;
    opacity: 0.7;
  }

  .tab:hover {
    opacity: 1;
  }

  .tab.active {
    opacity: 1;
    border-bottom-color: currentColor;
    font-weight: 600;
  }

  .groups {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .group {
    border: 1px solid color-mix(in srgb, currentColor 12%, transparent);
    border-radius: 8px;
    overflow: hidden;
  }

  .group.active-group {
    border-color: var(--success);
  }

  .group-head {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    border: none;
    background: color-mix(in srgb, currentColor 4%, transparent);
    font: inherit;
    cursor: pointer;
    text-align: left;
  }

  .group-head:hover {
    background: color-mix(in srgb, currentColor 7%, transparent);
  }

  .group-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
  }

  .group-meta {
    font-size: 0.8rem;
    opacity: 0.6;
    white-space: nowrap;
  }

  .models {
    width: 100%;
    border-collapse: collapse;
  }

  th,
  td {
    padding: 8px 10px;
    text-align: left;
    border-bottom: 1px solid color-mix(in srgb, currentColor 12%, transparent);
  }

  th {
    font-size: 0.85rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.6;
  }

  .name {
    display: block;
    font-weight: 500;
  }

  .quant {
    font-size: 0.8rem;
    opacity: 0.6;
  }

  .badge {
    display: inline-block;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 0.8rem;
    border: 1px solid color-mix(in srgb, currentColor 25%, transparent);
  }

  .backend-whisper,
  .status-downloaded {
    color: var(--success);
  }

  .backend-llama,
  .status-downloading {
    color: var(--warning);
  }

  .backend-ort {
    color: var(--info);
  }

  .status-error {
    color: var(--danger);
  }

  .status-none {
    opacity: 0.55;
  }

  .quality {
    white-space: nowrap;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .activate {
    font: inherit;
    cursor: pointer;
    padding: 2px 10px;
    border-radius: 6px;
    border: 1px solid color-mix(in srgb, currentColor 25%, transparent);
    background: none;
    color: inherit;
  }

  .activate:hover {
    opacity: 0.85;
  }

  .activate.active {
    color: var(--success);
    font-weight: 600;
    border-color: currentColor;
  }

  .active-row {
    background: color-mix(in srgb, currentColor 5%, transparent);
  }

  .warn {
    color: var(--warning);
  }

  .star {
    color: color-mix(in srgb, currentColor 25%, transparent);
  }

  .star.on {
    color: currentColor;
  }

  .error {
    color: var(--danger);
  }

  .empty {
    opacity: 0.6;
  }
</style>
