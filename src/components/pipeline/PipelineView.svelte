<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { onDestroy, onMount } from "svelte";
  import PreviewPane from "../preview/PreviewPane.svelte";
  import SubtitleEditor from "../editor/SubtitleEditor.svelte";
  import { showError } from "../common/ErrorToast.svelte";
  import { errMsg } from "../../lib/errors";
  import { t } from "../../lib/t";

  export interface PipelineSource {
    type: "audio" | "embedded";
    track_index?: number;
    stream_index?: number;
  }

  type StepId = "extract" | "transcribe" | "translate" | "format" | "export" | "done";

  interface ProgressPayload {
    job_id: string;
    step: StepId;
    pct: number;
    detail: string | null;
  }

  interface SummaryPayload {
    output_path: string;
    duration_secs: number;
    segments: number;
    source_lang: string;
    target_lang: string;
    kept_original: number;
  }

  interface FinishedPayload {
    job_id: string;
    ok: boolean;
    cancelled: boolean;
    error: { code: string; message: string; hint: string | null } | null;
    summary: SummaryPayload | null;
  }

  type Phase = "running" | "cancelled" | "done" | "error";

  const STEPS = $derived([
    { id: "extract", label: t("pipeline.stepExtract") },
    { id: "transcribe", label: t("pipeline.stepTranscribe") },
    { id: "translate", label: t("pipeline.stepTranslate") },
    { id: "format", label: t("pipeline.stepFormat") },
    { id: "export", label: t("pipeline.stepExport") },
  ] as { id: StepId; label: string }[]);

  let {
    jobId,
    inputPath,
    source,
    translate,
    onReset,
  }: {
    jobId: string;
    inputPath: string;
    source: PipelineSource;
    translate: boolean;
    onReset: () => void;
  } = $props();

  let phase = $state<Phase>("running");
  let activeStep = $state<StepId | null>(null);
  let pct = $state(0);
  let detail = $state<string | null>(null);
  let error = $state<string | null>(null);
  let summary = $state<SummaryPayload | null>(null);
  let showPreview = $state(false);
  let showEditor = $state(false);
  let unlisteners: (() => void)[] = [];

  // Etapas visíveis: legenda embutida pula o STT; `translate: false` pula a tradução.
  const visibleSteps = $derived(
    STEPS.filter((s) => {
      if (s.id === "transcribe" && source.type === "embedded") return false;
      if (s.id === "translate" && !translate) return false;
      return true;
    }),
  );

  const currentIndex = $derived(
    activeStep && activeStep !== "done" ? visibleSteps.findIndex((s) => s.id === activeStep) : -1,
  );

  onMount(() => {
    const p = listen<ProgressPayload>("pipeline-progress", (ev) => {
      if (ev.payload.job_id !== jobId) return;
      activeStep = ev.payload.step === "done" ? null : ev.payload.step;
      pct = ev.payload.pct;
      detail = ev.payload.detail;
    });
    const f = listen<FinishedPayload>("pipeline-finished", (ev) => {
      if (ev.payload.job_id !== jobId) return;
      if (ev.payload.cancelled) {
        phase = "cancelled";
      } else if (ev.payload.ok) {
        phase = "done";
        summary = ev.payload.summary;
      } else {
        phase = "error";
        const detail = ev.payload.error;
        error = detail ? errMsg(detail, "pipeline.errGeneric") : t("pipeline.errGeneric");
        showError(detail ?? t("pipeline.errGeneric"));
      }
    });
    void p.then((u) => unlisteners.push(u));
    void f.then((u) => unlisteners.push(u));

    invoke("run_pipeline", {
      job_id: jobId,
      input_path: inputPath,
      source,
      options: { translate },
    }).catch((e) => {
      phase = "error";
      error = errMsg(e, "pipeline.errStart");
      showError(e);
    });
  });

  onDestroy(() => {
    for (const u of unlisteners) u();
  });

  async function cancel(): Promise<void> {
    try {
      await invoke("cancel_pipeline", { job_id: jobId });
    } catch (e) {
      error = errMsg(e, "pipeline.errCancel");
      showError(e);
    }
  }

  async function reveal(): Promise<void> {
    if (!summary) return;
    try {
      await revealItemInDir(summary.output_path);
    } catch {
      // Sem ação — o caminho já está visível na tela.
    }
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
</script>

<section class="pipeline" aria-label={t("pipeline.aria")}>
  <h2 class="file">{inputPath.split("/").pop()}</h2>

  <ol class="stepper">
    {#each visibleSteps as step, i (step.id)}
      <li
        class:active={phase === "running" && currentIndex === i}
        class:done={phase === "done" ||
          (phase === "running" && currentIndex >= 0 && i < currentIndex)}
        class:future={phase === "running" && currentIndex >= 0 && i > currentIndex}
      >
        <span class="dot" aria-hidden="true"></span>
        <span class="label">{step.label}</span>
      </li>
    {/each}
  </ol>

  {#if phase === "running"}
    <div class="bar" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
      <div class="fill" style="width: {pct}%"></div>
    </div>
    <div class="row">
      <span class="pct">{pct}%</span>
      {#if detail}<span class="detail">{detail}</span>{/if}
      <button type="button" class="cancel" onclick={cancel}>{t("pipeline.cancel")}</button>
    </div>
  {/if}

  {#if phase === "cancelled"}
    <p class="status-cancelled" role="status">{t("pipeline.cancelled")}</p>
  {/if}

  {#if phase === "error"}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if phase === "done" && summary}
    <div class="summary" role="status">
      <h3>{t("pipeline.done")}</h3>
      <dl>
        <div>
          <dt>{t("pipeline.duration")}</dt>
          <dd>{fmtDuration(summary.duration_secs)}</dd>
        </div>
        <div>
          <dt>{t("pipeline.subtitles")}</dt>
          <dd>{summary.segments}</dd>
        </div>
        <div>
          <dt>{t("pipeline.languages")}</dt>
          <dd>{summary.source_lang} → {summary.target_lang}</dd>
        </div>
        {#if summary.kept_original > 0}
          <div>
            <dt>{t("pipeline.keptOriginal")}</dt>
            <dd>{summary.kept_original}</dd>
          </div>
        {/if}
        <div class="output">
          <dt>{t("pipeline.output")}</dt>
          <dd>{summary.output_path}</dd>
        </div>
      </dl>
      <div class="row">
        <button type="button" onclick={reveal}>{t("pipeline.reveal")}</button>
        <button type="button" onclick={() => (showPreview = true)}>{t("pipeline.preview")}</button>
        <button type="button" onclick={() => (showEditor = true)}>{t("pipeline.edit")}</button>
        <button type="button" class="primary" onclick={onReset}>
          {t("pipeline.another")}
        </button>
      </div>
    </div>

    {#if showPreview}
      <PreviewPane videoPath={inputPath} srtPath={summary.output_path} />
    {/if}

    {#if showEditor}
      <SubtitleEditor videoPath={inputPath} srtPath={summary.output_path} />
    {/if}
  {/if}

  {#if phase !== "running"}
    <button type="button" class="back" onclick={onReset}>{t("pipeline.back")}</button>
  {/if}
</section>

<style>
  .pipeline {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 640px;
  }

  .file {
    margin: 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
    word-break: break-all;
  }

  .stepper {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .stepper li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-muted);
    font-size: var(--font-size-sm);
  }

  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--border);
  }

  .stepper li.active {
    border-color: var(--accent);
    color: var(--text);
  }

  .stepper li.active .dot {
    background: var(--accent);
  }

  .stepper li.done {
    color: var(--success);
  }

  .stepper li.done .dot {
    background: var(--success);
  }

  .stepper li.future {
    opacity: 0.5;
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

  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .pct {
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    font-size: var(--font-size-sm);
    min-width: 36px;
  }

  .detail {
    color: var(--text-muted);
    font-size: var(--font-size-sm);
  }

  .cancel {
    margin-left: auto;
  }

  .error {
    color: var(--danger);
    margin: 0;
  }

  .status-cancelled {
    color: var(--warning);
    margin: 0;
  }

  .summary {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .summary h3 {
    margin: 0;
    font-size: var(--font-size-lg);
    color: var(--success);
  }

  dl {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  dl div {
    display: flex;
    gap: var(--space-3);
  }

  dt {
    color: var(--text-muted);
    min-width: 140px;
  }

  dd {
    margin: 0;
    word-break: break-all;
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

  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: white;
  }

  .back {
    align-self: flex-start;
  }
</style>
