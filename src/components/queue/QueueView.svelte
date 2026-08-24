<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { onDestroy, onMount } from "svelte";
  import PreviewPane from "../preview/PreviewPane.svelte";
  import SubtitleEditor from "../editor/SubtitleEditor.svelte";
  import StatsPanel, { type JobStats } from "../stats/StatsPanel.svelte";
  import JobDetails from "./JobDetails.svelte";
  import { showError } from "../common/ErrorToast.svelte";
  import { errMsg } from "../../lib/errors";
  import { t } from "../../lib/t";

  type QueueState = "pending" | "running" | "done" | "error" | "cancelled";
  type StepId = "extract" | "transcribe" | "translate" | "format" | "export" | "done";

  interface Summary {
    output_path: string;
    duration_secs: number;
    segments: number;
    source_lang: string;
    target_lang: string;
    kept_original: number;
    stats: JobStats;
  }

  interface QueueItem {
    id: string;
    input_path: string;
    state: QueueState;
    step: StepId | null;
    pct: number;
    detail: string | null;
    summary: Summary | null;
    error: { code: string; message: string; hint: string | null } | null;
  }

  // Rótulo da etapa atual do item em execução (reutiliza os mesmos textos do
  // stepper da 4.3). `$derived` para trocar idioma sem refresh.
  const STEP_LABELS = $derived({
    extract: t("pipeline.stepExtract"),
    transcribe: t("pipeline.stepTranscribe"),
    translate: t("pipeline.stepTranslate"),
    format: t("pipeline.stepFormat"),
    export: t("pipeline.stepExport"),
    done: "",
  } as Record<StepId, string>);

  let items = $state<QueueItem[]>([]);
  let open = $state<Record<string, "preview" | "editor" | null>>({});
  let detailsOpen = $state<Record<string, boolean>>({});
  let unlisteners: (() => void)[] = [];

  onMount(() => {
    // `queue-updated` é a fonte de verdade do estado dos itens; `pipeline-progress`
    // dá o avanço em tempo real do item em execução; `pipeline-finished` cobre o
    // caso de o evento de fila chegar antes do listener (refresh defensivo).
    const u = listen<QueueItem[]>("queue-updated", (ev) => {
      items = ev.payload;
    });
    const p = listen<{ job_id: string; step: StepId; pct: number; detail: string | null }>(
      "pipeline-progress",
      (ev) => {
        const it = items.find((i) => i.id === ev.payload.job_id);
        if (it && it.state === "running") {
          it.step = ev.payload.step;
          it.pct = ev.payload.pct;
          it.detail = ev.payload.detail;
        }
      },
    );
    const f = listen("pipeline-finished", () => {
      void refresh();
    });
    void Promise.all([u, p, f]).then((all) => unlisteners.push(...all));
    void refresh();
  });

  onDestroy(() => {
    for (const u of unlisteners) u();
  });

  async function refresh(): Promise<void> {
    try {
      items = await invoke<QueueItem[]>("queue_list");
    } catch (e) {
      showError(e);
    }
  }

  function fileName(path: string): string {
    return path.split("/").pop() ?? path;
  }

  function fmtDuration(secs: number): string {
    if (!secs) return "—";
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = Math.floor(secs % 60);
    const mm = String(m).padStart(2, "0");
    const ss = String(s).padStart(2, "0");
    return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
  }

  function isRunning(item: QueueItem): boolean {
    return item.state === "running";
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
      const rest = { ...open };
      delete rest[item.id];
      open = rest;
    } catch (e) {
      showError(e);
    }
  }

  async function reveal(item: QueueItem): Promise<void> {
    if (!item.summary) return;
    try {
      await revealItemInDir(item.summary.output_path);
    } catch {
      // Sem ação — o caminho já está visível na tela.
    }
  }

  function toggle(item: QueueItem, mode: "preview" | "editor"): void {
    open = { ...open, [item.id]: open[item.id] === mode ? null : mode };
  }

  function toggleDetails(id: string): void {
    detailsOpen = { ...detailsOpen, [id]: !detailsOpen[id] };
  }
</script>

<section class="queue" aria-label={t("queue.aria")}>
  <h2>{t("queue.title")}</h2>

  {#if items.length === 0}
    <p class="empty">{t("queue.empty")}</p>
  {:else}
    <ul class="items">
      {#each items as item (item.id)}
        <li class="item item-{item.state}">
          <div class="head">
            <span class="name">{fileName(item.input_path)}</span>
            <span class="badge badge-{item.state}">{t(`queue.status.${item.state}`)}</span>
          </div>

          {#if isRunning(item)}
            <div class="progress">
              <div class="step">
                <span class="step-label">
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
            </div>
          {/if}

          {#if item.error}
            <p class="error" role="alert">{errMsg(item.error, "queue.errGeneric")}</p>
          {/if}

          {#if item.summary}
            <dl class="summary">
              <div>
                <dt>{t("queue.output")}</dt>
                <dd>{item.summary.output_path}</dd>
              </div>
              <div>
                <dt>{t("queue.duration")}</dt>
                <dd>{fmtDuration(item.summary.duration_secs)}</dd>
              </div>
              <div>
                <dt>{t("queue.subtitles")}</dt>
                <dd>{item.summary.segments}</dd>
              </div>
              {#if item.summary.target_lang}
                <div>
                  <dt>{t("queue.languages")}</dt>
                  <dd>{item.summary.source_lang} → {item.summary.target_lang}</dd>
                </div>
              {/if}
            </dl>
            <StatsPanel stats={item.summary.stats} />
          {/if}

          <div class="actions">
            <button
              type="button"
              class:active={!!detailsOpen[item.id]}
              onclick={() => toggleDetails(item.id)}
            >
              {detailsOpen[item.id] ? t("queue.detailsHide") : t("queue.detailsShow")}
            </button>
            {#if isRunning(item)}
              <button type="button" onclick={() => cancel(item)}>{t("queue.cancel")}</button>
            {:else if item.state === "done"}
              <button type="button" onclick={() => reveal(item)}>{t("queue.reveal")}</button>
              <button type="button" onclick={() => toggle(item, "preview")}>
                {t("queue.preview")}
              </button>
              <button type="button" onclick={() => toggle(item, "editor")}>
                {t("queue.edit")}
              </button>
            {/if}
            {#if !isRunning(item)}
              <button type="button" class="danger" onclick={() => remove(item)}>
                {t("queue.remove")}
              </button>
            {/if}
          </div>

          {#if detailsOpen[item.id]}
            {#key item.id}
              <JobDetails {item} />
            {/key}
          {/if}

          {#if item.summary && open[item.id] === "preview"}
            <PreviewPane videoPath={item.input_path} srtPath={item.summary.output_path} />
          {/if}
          {#if item.summary && open[item.id] === "editor"}
            <SubtitleEditor videoPath={item.input_path} srtPath={item.summary.output_path} />
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .queue {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 720px;
  }

  .queue h2 {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
  }

  .empty {
    color: var(--text-muted);
    margin: 0;
  }

  .items {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .item {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .item-running {
    border-color: var(--accent);
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
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

  .badge-pending {
    color: var(--text);
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
    background: var(--surface-2);
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

  .summary {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .summary div {
    display: flex;
    gap: var(--space-3);
  }

  dt {
    color: var(--text-muted);
    min-width: 100px;
  }

  dd {
    margin: 0;
    word-break: break-all;
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

  button.active {
    border-color: var(--accent);
    color: var(--accent);
  }

  .danger:hover {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
