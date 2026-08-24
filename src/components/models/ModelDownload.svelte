<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy, onMount } from "svelte";
  import { showError } from "../common/ErrorToast.svelte";
  import { errMsg } from "../../lib/errors";
  import { t } from "../../lib/t";

  type CacheStatus = "downloading" | "downloaded" | "error";

  interface ModelInfo {
    id: string;
    name: string;
  }

  interface ProgressPayload {
    model_id: string;
    file: string;
    bytes: number;
    total: number;
  }

  interface FinishedPayload {
    model_id: string;
    ok: boolean;
  }

  let {
    model,
    status,
    onStatusChange,
  }: {
    model: ModelInfo;
    status: CacheStatus | null;
    onStatusChange: () => void;
  } = $props();

  let downloading = $state(false);
  let bytes = $state(0);
  let total = $state(0);
  let error: string | null = $state(null);
  let unlisteners: (() => void)[] = [];

  const pct = $derived(total > 0 ? Math.min(100, Math.round((bytes / total) * 100)) : 0);

  onMount(() => {
    const p = listen<ProgressPayload>("model-download-progress", (ev) => {
      if (ev.payload.model_id !== model.id) return;
      downloading = true;
      bytes = ev.payload.bytes;
      total = ev.payload.total;
    });
    const f = listen<FinishedPayload>("model-download-finished", (ev) => {
      if (ev.payload.model_id !== model.id) return;
      downloading = false;
      bytes = 0;
      total = 0;
      error = ev.payload.ok ? null : t("download.errFailed");
      if (!ev.payload.ok) showError(error ?? t("download.errFailed"));
      onStatusChange();
    });
    void p.then((u) => unlisteners.push(u));
    void f.then((u) => unlisteners.push(u));
  });

  onDestroy(() => {
    for (const u of unlisteners) u();
  });

  async function start(): Promise<void> {
    error = null;
    downloading = true;
    try {
      await invoke("download_model", { id: model.id });
      onStatusChange();
    } catch (e) {
      downloading = false;
      error = errMsg(e, "download.errStart");
      showError(e);
      onStatusChange();
    }
  }

  async function cancel(): Promise<void> {
    try {
      await invoke("cancel_download", { id: model.id });
    } catch (e) {
      error = errMsg(e, "download.errCancel");
      showError(e);
    }
    downloading = false;
    onStatusChange();
  }

  async function remove(): Promise<void> {
    if (!window.confirm(t("download.confirmRemove", { name: model.name }))) return;
    try {
      await invoke("delete_model", { id: model.id });
    } catch (e) {
      error = errMsg(e, "download.errRemove");
      showError(e);
    }
    onStatusChange();
  }
</script>

{#if downloading}
  <div class="dl">
    <div class="bar" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
      <div class="fill" style="width: {pct}%"></div>
    </div>
    <div class="dl-actions">
      <span class="pct">{pct}%</span>
      <button type="button" onclick={cancel}>{t("download.cancel")}</button>
    </div>
  </div>
{:else}
  <div class="dl-actions">
    {#if status === "downloaded"}
      <button type="button" class="danger" onclick={remove}>{t("download.remove")}</button>
    {:else}
      <button type="button" onclick={start}>{t("download.download")}</button>
    {/if}
  </div>
{/if}
{#if error}
  <p class="error" role="alert">{error}</p>
{/if}

<style>
  .dl {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 160px;
  }

  .bar {
    height: 8px;
    border-radius: 4px;
    background: color-mix(in srgb, currentColor 15%, transparent);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: currentColor;
    transition: width 0.2s;
  }

  .dl-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .pct {
    font-size: 0.8rem;
    opacity: 0.7;
    min-width: 36px;
  }

  .danger {
    color: var(--danger);
  }

  button {
    font: inherit;
    cursor: pointer;
    padding: 2px 10px;
    border-radius: 6px;
    border: 1px solid color-mix(in srgb, currentColor 25%, transparent);
    background: none;
    color: inherit;
  }

  button:hover {
    opacity: 0.85;
  }

  .error {
    color: var(--danger);
    font-size: 0.8rem;
    margin: 2px 0 0;
  }
</style>
