<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy, onMount } from "svelte";
  import { showError } from "../common/ErrorToast.svelte";
  import { errMsg } from "../../lib/errors";
  import { t } from "../../lib/t";

  type QueueState = "pending" | "running" | "done" | "error" | "cancelled";
  type StepId = "extract" | "transcribe" | "translate" | "format" | "export" | "done";
  type Origin = "local" | "remote";

  interface NetStatus {
    running: boolean;
    port: number;
    url: string | null;
  }

  interface NetInfo {
    name: string;
    host: string;
    port: number;
    protocol: number;
    version: string;
    url: string;
    tailscale_host?: string | null;
    tailscale_url?: string | null;
  }

  interface QueueItem {
    id: string;
    input_path: string;
    state: QueueState;
    step: StepId | null;
    pct: number;
    detail: string | null;
    summary: { segments: number; output_path: string } | null;
    error: { code: string; message: string; hint: string | null } | null;
    origin: Origin;
    anime_key?: string;
    episode?: number;
    client_job_id?: string;
  }

  const STEP_LABELS = $derived({
    extract: t("pipeline.stepExtract"),
    transcribe: t("pipeline.stepTranscribe"),
    translate: t("pipeline.stepTranslate"),
    format: t("pipeline.stepFormat"),
    export: t("pipeline.stepExport"),
    done: "",
  } as Record<StepId, string>);

  let status = $state<NetStatus | null>(null);
  let info = $state<NetInfo | null>(null);
  let qrSvg = $state("");
  let qrError = $state("");
  let qrTailscaleSvg = $state("");
  let qrTailscaleError = $state("");
  let portInput = $state<string | number>("");
  let busy = $state(false);
  let notice = $state("");
  let items = $state<QueueItem[]>([]);
  let unlisteners: (() => void)[] = [];

  const remoteItems = $derived(items.filter((i) => i.origin === "remote"));
  const qrSrc = $derived(qrSvg ? `data:image/svg+xml,${encodeURIComponent(qrSvg)}` : "");
  const qrTailscaleSrc = $derived(
    qrTailscaleSvg ? `data:image/svg+xml,${encodeURIComponent(qrTailscaleSvg)}` : "",
  );

  onMount(() => {
    const u = listen<QueueItem[]>("queue-updated", (ev) => {
      items = ev.payload;
    });
    const f = listen("pipeline-finished", () => {
      void refreshQueue();
    });
    void Promise.all([u, f]).then((all) => unlisteners.push(...all));
    void refreshAll();
  });

  onDestroy(() => {
    for (const u of unlisteners) u();
  });

  async function refreshAll(): Promise<void> {
    await Promise.all([refreshStatus(), refreshQueue()]);
  }

  async function refreshStatus(): Promise<void> {
    try {
      const s = await invoke<NetStatus>("net_status");
      status = s;
      portInput = s.port;
      if (s.running) {
        info = await invoke<NetInfo | null>("net_info");
        try {
          qrSvg = await invoke<string>("net_qr_svg");
          qrError = "";
        } catch (e) {
          qrSvg = "";
          qrError = String(e);
        }
        if (info?.tailscale_url) {
          try {
            qrTailscaleSvg = await invoke<string>("net_qr_svg_for", {
              url: info.tailscale_url,
            });
            qrTailscaleError = "";
          } catch (e) {
            qrTailscaleSvg = "";
            qrTailscaleError = String(e);
          }
        } else {
          qrTailscaleSvg = "";
        }
      } else {
        info = null;
        qrSvg = "";
        qrTailscaleSvg = "";
      }
    } catch (e) {
      showError(e);
    }
  }

  async function refreshQueue(): Promise<void> {
    try {
      items = await invoke<QueueItem[]>("queue_list");
    } catch (e) {
      showError(e);
    }
  }

  async function copyAddress(): Promise<void> {
    if (!status?.url) return;
    try {
      await navigator.clipboard.writeText(status.url);
      notice = t("net.copied");
    } catch {
      notice = status.url;
    }
    setTimeout(() => (notice = ""), 2500);
  }

  async function copyTailscale(): Promise<void> {
    if (!info?.tailscale_url) return;
    try {
      await navigator.clipboard.writeText(info.tailscale_url);
      notice = t("net.copied");
    } catch {
      notice = info.tailscale_url;
    }
    setTimeout(() => (notice = ""), 2500);
  }

  async function savePort(): Promise<void> {
    const port = Number.parseInt(String(portInput), 10);
    if (!Number.isFinite(port) || port < 1024 || port > 65535) {
      notice = t("net.portInvalid");
      return;
    }
    busy = true;
    try {
      notice = await invoke<string>("net_set_port", { port });
    } catch (e) {
      showError(e);
    } finally {
      busy = false;
    }
  }

  async function cancel(item: QueueItem): Promise<void> {
    try {
      await invoke("queue_cancel", { id: item.id });
    } catch (e) {
      showError(e);
    }
  }

  async function remove(item: QueueItem): Promise<void> {
    try {
      await invoke("queue_remove", { id: item.id });
    } catch (e) {
      showError(e);
    }
  }

  function isRunning(item: QueueItem): boolean {
    return item.state === "running";
  }

  function remoteName(item: QueueItem): string {
    if (item.anime_key) {
      return item.episode ? `${item.anime_key} · EP${item.episode}` : item.anime_key;
    }
    return item.client_job_id ?? item.id;
  }
</script>

<section class="net" aria-label={t("net.aria")}>
  <h2>{t("net.title")}</h2>
  <p class="hint">{t("net.hint")}</p>

  <div class="card">
    <div class="row">
      <span class="dot" class:on={status?.running}></span>
      <strong>
        {status?.running ? t("net.running") : t("net.stopped")}
      </strong>
      {#if status?.url}
        <code class="addr">{status.url}</code>
        <button type="button" onclick={copyAddress}>{t("net.copy")}</button>
      {/if}
    </div>
    {#if info}
      <p class="meta">
        {t("net.pcName")}: <strong>{info.name}</strong> · v{info.version} · protocolo v{info.protocol}
      </p>
    {/if}
    {#if notice}
      <p class="notice">{notice}</p>
    {/if}

    <div class="port">
      <label for="net-port">{t("net.port")}</label>
      <input
        id="net-port"
        type="number"
        min="1024"
        max="65535"
        bind:value={portInput}
        disabled={busy}
      />
      <button type="button" onclick={savePort} disabled={busy}>{t("net.savePort")}</button>
    </div>
  </div>

  <div class="card qr-card">
    <h3>{t("net.qrTitle")}</h3>
    <p class="hint">{t("net.qrHint")}</p>
    {#if qrSrc}
      <div class="qr" aria-label={t("net.qrTitle")}>
        <!-- QR gerado no backend (Rust) a partir da URL de pareamento. -->
        <img src={qrSrc} alt={t("net.qrTitle")} />
      </div>
    {:else}
      <p class="warn">{qrError || t("net.qrUnavailable")}</p>
    {/if}

    {#if info?.tailscale_url}
      <hr class="sep" />
      <h3>{t("net.tailscaleTitle")}</h3>
      <p class="hint">{t("net.tailscaleHint")}</p>
      <div class="row">
        <code class="addr">{info.tailscale_url}</code>
        <button type="button" onclick={copyTailscale}>{t("net.copy")}</button>
      </div>
      {#if qrTailscaleSrc}
        <div class="qr" aria-label={t("net.tailscaleTitle")}>
          <img src={qrTailscaleSrc} alt={t("net.tailscaleTitle")} />
        </div>
      {:else}
        <p class="warn">{qrTailscaleError || t("net.qrUnavailable")}</p>
      {/if}
    {/if}
  </div>

  <div class="card">
    <div class="row between">
      <h3>{t("net.remoteJobs")}</h3>
      <button type="button" onclick={refreshQueue}>{t("net.refresh")}</button>
    </div>
    <p class="hint">{t("net.remoteHint")}</p>
    {#if remoteItems.length === 0}
      <p class="empty">{t("net.noRemote")}</p>
    {:else}
      <ul class="jobs">
        {#each remoteItems as item (item.id)}
          <li class="job job-{item.state}">
            <div class="row between">
              <span class="name">{remoteName(item)}</span>
              <span class="badge badge-{item.state}">{t(`queue.status.${item.state}`)}</span>
            </div>
            {#if isRunning(item)}
              <div class="step">
                <span>
                  {#if item.step && STEP_LABELS[item.step]}
                    {STEP_LABELS[item.step]}
                    {#if item.detail}<span class="detail">· {item.detail}</span>{/if}
                  {/if}
                </span>
                <span class="pct">{item.pct}%</span>
              </div>
              <div
                class="bar"
                role="progressbar"
                aria-valuenow={item.pct}
                aria-valuemin={0}
                aria-valuemax={100}
              >
                <div class="fill" style="width: {item.pct}%"></div>
              </div>
            {/if}
            {#if item.error}
              <p class="error" role="alert">{errMsg(item.error, "queue.errGeneric")}</p>
            {/if}
            <div class="actions">
              {#if isRunning(item)}
                <button type="button" onclick={() => cancel(item)}>{t("queue.cancel")}</button>
              {:else}
                <button type="button" class="danger" onclick={() => remove(item)}>
                  {t("queue.remove")}
                </button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</section>

<style>
  .net {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 720px;
  }

  .net h2 {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
  }

  .net h3 {
    margin: 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }

  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--font-size-sm);
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .row.between {
    justify-content: space-between;
  }

  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--danger);
    flex-shrink: 0;
  }

  .dot.on {
    background: var(--success);
  }

  .addr {
    font-family: var(--font-mono, monospace);
    background: var(--surface-2);
    padding: 2px var(--space-2);
    border-radius: var(--radius-sm);
    word-break: break-all;
  }

  .meta,
  .notice,
  .warn {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--text-muted);
  }

  .notice {
    color: var(--accent);
  }

  .warn {
    color: var(--warning);
  }

  .sep {
    border: none;
    border-top: 1px solid var(--border);
    width: 100%;
    margin: var(--space-2) 0;
  }

  .port {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .port input {
    width: 120px;
    font: inherit;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
  }

  .qr {
    width: 260px;
    height: 260px;
    padding: var(--space-2);
    background: #ffffff;
    border-radius: var(--radius-sm);
    align-self: center;
  }

  .qr img {
    width: 100%;
    height: 100%;
    display: block;
  }

  .jobs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .job {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
  }

  .job-running {
    border-color: var(--accent);
  }

  .name {
    font-weight: var(--font-weight-semibold);
    word-break: break-all;
  }

  .badge {
    font-size: var(--font-size-sm);
    padding: 2px var(--space-2);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    color: var(--text-muted);
    white-space: nowrap;
  }

  .badge-running {
    border-color: var(--accent);
    color: var(--accent);
  }

  .badge-done {
    border-color: var(--success);
    color: var(--success);
  }

  .badge-error {
    border-color: var(--danger);
    color: var(--danger);
  }

  .badge-cancelled {
    border-color: var(--warning);
    color: var(--warning);
  }

  .step {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
    font-size: var(--font-size-sm);
    color: var(--text-muted);
  }

  .detail {
    opacity: 0.8;
  }

  .pct {
    font-variant-numeric: tabular-nums;
  }

  .bar {
    height: 10px;
    border-radius: 5px;
    background: var(--surface);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s;
  }

  .error {
    color: var(--danger);
    margin: 0;
  }

  .empty {
    margin: 0;
    color: var(--text-muted);
  }

  .actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  button {
    font: inherit;
    cursor: pointer;
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
  }

  button:hover {
    border-color: var(--accent);
  }

  button.danger:hover {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
