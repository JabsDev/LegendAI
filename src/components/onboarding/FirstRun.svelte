<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import ModelDownload from "../models/ModelDownload.svelte";
  import { showError } from "../common/ErrorToast.svelte";
  import { errMsg } from "../../lib/errors";
  import { t } from "../../lib/t";

  type ModelKind = "stt" | "translation";
  type GpuKind = "cuda" | "rocm" | "metal";
  type CacheStatus = "downloading" | "downloaded" | "error";

  interface ModelInfo {
    id: string;
    kind: ModelKind;
    name: string;
    size_mb: number;
  }

  interface HardwareInfo {
    ram_gb: number;
    cpu_threads: number;
    gpu: GpuKind | null;
  }

  interface OnboardingInfo {
    first_run: boolean;
    hardware: HardwareInfo;
    tier: "tier1" | "tier2" | "tier3";
    recommendations: { stt: ModelInfo[]; translation: ModelInfo[] };
  }

  interface StatusRow {
    model_id: string;
    status: CacheStatus | null;
  }

  let { done }: { done: () => void } = $props();

  let loading = $state(true);
  let loadError: string | null = $state(null);
  let info = $state<OnboardingInfo | null>(null);
  let statuses = $state<Record<string, CacheStatus | null>>({});
  let downloadingRecommended = $state(false);

  const stt = $derived(info?.recommendations.stt[0] ?? null);
  const translation = $derived(info?.recommendations.translation[0] ?? null);

  const TIER_LABEL = $derived<Record<string, string>>({
    tier1: t("onboarding.tier1"),
    tier2: t("onboarding.tier2"),
    tier3: t("onboarding.tier3"),
  });

  const GPU_LABEL = $derived<Record<string, string>>({
    cuda: "CUDA",
    rocm: "ROCm",
    metal: "Metal",
  });

  onMount(() => {
    void load();
  });

  async function load(): Promise<void> {
    try {
      info = await invoke<OnboardingInfo>("get_onboarding");
      await refreshStatuses();
    } catch (e) {
      loadError = errMsg(e, "onboarding.errLoad");
      showError(e);
    } finally {
      loading = false;
    }
  }

  async function refreshStatuses(): Promise<void> {
    try {
      const cacheStatus = await invoke<StatusRow[]>("list_cache_status");
      statuses = Object.fromEntries(cacheStatus.map((s) => [s.model_id, s.status]));
    } catch {
      // status é só um extra; onboarding não quebra sem ele.
    }
  }

  function formatSize(mb: number): string {
    return mb >= 1024
      ? t("models.sizeGb", { value: (mb / 1024).toFixed(1) })
      : t("models.sizeMb", { value: mb });
  }

  function gpuLabel(gpu: GpuKind | null): string {
    return gpu ? GPU_LABEL[gpu] : t("onboarding.cpuOnly");
  }

  // Baixa os dois recomendados (STT + tradução) via 2.9; o progresso de cada um
  // é refletido pelos `ModelDownload` abaixo via eventos Tauri.
  async function downloadRecommended(): Promise<void> {
    const ids = [stt, translation].filter(Boolean).map((m) => m!.id);
    if (ids.length === 0) return;
    downloadingRecommended = true;
    try {
      await Promise.all(ids.map((id) => invoke("download_model", { id })));
    } catch (e) {
      showError(e);
    } finally {
      downloadingRecommended = false;
      await refreshStatuses();
    }
  }
</script>

<div class="onboarding" role="dialog" aria-modal="true" aria-labelledby="onboarding-title">
  <div class="card">
    <h1 id="onboarding-title">{t("onboarding.welcome")}</h1>
    <p class="subtitle">{t("onboarding.welcomeHint")}</p>

    {#if loading}
      <p class="muted">{t("onboarding.detecting")}</p>
    {:else if loadError}
      <p class="error" role="alert">{loadError}</p>
    {:else if info}
      <section class="hardware" aria-label={t("onboarding.hardwareAria")}>
        <span class="tier">{TIER_LABEL[info.tier]}</span>
        <span class="spec">{t("onboarding.ram", { value: info.hardware.ram_gb })}</span>
        <span class="spec">{t("onboarding.cpu", { value: info.hardware.cpu_threads })}</span>
        <span class="spec">{gpuLabel(info.hardware.gpu)}</span>
      </section>

      <section class="recs" aria-label={t("onboarding.recsAria")}>
        {#if stt}
          <div class="rec">
            <div class="rec-info">
              <strong>{stt.name}</strong>
              <span class="muted">{formatSize(stt.size_mb)} · {t("onboarding.forStt")}</span>
            </div>
            <ModelDownload
              model={stt}
              status={statuses[stt.id] ?? null}
              onStatusChange={refreshStatuses}
            />
          </div>
        {/if}
        {#if translation}
          <div class="rec">
            <div class="rec-info">
              <strong>{translation.name}</strong>
              <span class="muted"
                >{formatSize(translation.size_mb)} · {t("onboarding.forTranslation")}</span
              >
            </div>
            <ModelDownload
              model={translation}
              status={statuses[translation.id] ?? null}
              onStatusChange={refreshStatuses}
            />
          </div>
        {/if}
      </section>

      <div class="actions">
        <button
          type="button"
          class="primary"
          disabled={downloadingRecommended}
          onclick={downloadRecommended}
        >
          {t("onboarding.downloadRecommended")}
        </button>
        <button type="button" onclick={done}>{t("onboarding.skip")}</button>
      </div>
    {/if}
  </div>
</div>

<style>
  .onboarding {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: var(--surface);
    padding: var(--space-5);
  }

  .card {
    max-width: 480px;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-5);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-2);
  }

  .card h1 {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
  }

  .subtitle {
    margin: 0;
    color: var(--text-muted);
  }

  .muted {
    color: var(--text-muted);
  }

  .hardware {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
  }

  .tier {
    font-weight: var(--font-weight-semibold);
    color: var(--accent);
    border: 1px solid currentColor;
    border-radius: 999px;
    padding: 2px 12px;
  }

  .spec {
    color: var(--text-muted);
    font-size: var(--font-size-sm);
  }

  .recs {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .rec {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .rec-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .actions {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-1);
  }

  button {
    font: inherit;
    cursor: pointer;
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: none;
    color: inherit;
  }

  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: white;
    font-weight: var(--font-weight-semibold);
  }

  button:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .error {
    color: var(--danger);
  }
</style>
