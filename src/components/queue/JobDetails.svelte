<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy, onMount } from "svelte";
  import { t } from "../../lib/t";

  type StepId = "extract" | "transcribe" | "translate" | "format" | "export" | "done";

  interface QueueItemLike {
    id: string;
    state: string;
    step: StepId | null;
    pct: number;
    detail: string | null;
    summary: { duration_secs: number; stats?: { processing_secs: number } } | null;
  }

  let { item }: { item: QueueItemLike } = $props();

  interface LogLine {
    at: string;
    step: string;
    pct: number;
    detail: string | null;
  }

  let logs = $state<LogLine[]>([]);
  let samples = $state<{ t: number; pct: number }[]>([]);
  let elapsed = $state(0);
  let startMs = $state<number | null>(null);
  let tick: ReturnType<typeof setInterval> | null = null;
  let unlisteners: (() => void)[] = [];
  let termEl: HTMLDivElement | null = $state(null);

  const STEP_LABELS = $derived({
    extract: t("pipeline.stepExtract"),
    transcribe: t("pipeline.stepTranscribe"),
    translate: t("pipeline.stepTranslate"),
    format: t("pipeline.stepFormat"),
    export: t("pipeline.stepExport"),
    done: "done",
  } as Record<string, string>);

  function fmtClock(s: number): string {
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const sec = Math.floor(s % 60);
    const mm = String(m).padStart(2, "0");
    const ss = String(sec).padStart(2, "0");
    return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
  }

  function nowLabel(): string {
    const d = new Date();
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}:${String(d.getSeconds()).padStart(2, "0")}`;
  }

  function pushLog(step: StepId | null, pct: number, detail: string | null) {
    const label = step ? (STEP_LABELS[step] ?? step) : "—";
    logs = [...logs, { at: nowLabel(), step: label, pct, detail }];
    if (logs.length > 300) logs = logs.slice(-300);
    queueMicrotask(() => {
      if (termEl) termEl.scrollTop = termEl.scrollHeight;
    });
  }

  function recordSample(pct: number) {
    const now = Date.now();
    if (startMs === null) startMs = now;
    samples = [...samples, { t: now, pct }].slice(-20);
    elapsed = (now - startMs) / 1000;
  }

  // Métricas derivadas — "tokens/s" pra STT/tradução vira %/s + seg/s + lotes/s
  const pctPerSec = $derived.by(() => {
    if (samples.length < 2 || elapsed < 1) return 0;
    const first = samples[0];
    const last = samples[samples.length - 1];
    const dt = (last.t - first.t) / 1000;
    if (dt < 0.5) return 0;
    return (last.pct - first.pct) / dt;
  });

  const etaSecs = $derived.by(() => {
    if (pctPerSec <= 0.05) return null;
    const remain = 100 - item.pct;
    return remain / pctPerSec;
  });

  // traduz detalhe "3/10 lotes" -> lotes/s
  const batchInfo = $derived.by(() => {
    if (!item.detail) return null;
    const m = item.detail.match(/(\d+)\s*\/\s*(\d+)/);
    if (!m) return null;
    return { done: parseInt(m[1], 10), total: parseInt(m[2], 10) };
  });

  const lotesPerSec = $derived.by(() => {
    if (!batchInfo || samples.length < 2 || elapsed < 1) return 0;
    // aproxima: pct cobre lotes linearmente
    const done = batchInfo.done;
    // lotes/s ≈ done / elapsed
    return done / elapsed || 0;
  });

  // realtime factor quando já tem duração e está rodando
  const realtimeFactor = $derived.by(() => {
    if (!item.summary?.duration_secs) return null;
    if (elapsed <= 0) return null;
    return item.summary.duration_secs / elapsed;
  });

  let lastKey = $state("");

  // Observa mudanças do item (patch via queue-updated/pipeline-progress)
  $effect(() => {
    const key = `${item.step ?? ""}:${item.pct}:${item.detail ?? ""}`;
    if (key === lastKey) return;
    lastKey = key;
    // ignora pending sem progresso
    if (item.state === "pending" && item.pct === 0 && !item.step) return;
    pushLog(item.step, item.pct, item.detail);
    if (item.state === "running") recordSample(item.pct);
  });

  // Se o job já está rodando quando o painel abre, semeia um log inicial
  onMount(() => {
    if (item.step || item.pct > 0 || item.detail) {
      pushLog(item.step, item.pct, item.detail);
      if (item.state === "running") recordSample(item.pct);
    }

    // Escuta pipeline-progress e pipeline-log (quando backend enviar)
    const p = listen<{ job_id: string; step: StepId; pct: number; detail: string | null }>(
      "pipeline-progress",
      (ev) => {
        if (ev.payload.job_id !== item.id) return;
        // $effect já cobre, mas garante log mesmo se item ainda não patchado
      },
    );
    const l = listen<{ job_id: string; line: string }>("pipeline-log", (ev) => {
      if (ev.payload.job_id !== item.id) return;
      logs = [...logs, { at: nowLabel(), step: "log", pct: item.pct, detail: ev.payload.line }];
      if (logs.length > 300) logs = logs.slice(-300);
      queueMicrotask(() => {
        if (termEl) termEl.scrollTop = termEl.scrollHeight;
      });
    });
    void Promise.all([p, l]).then((all) => unlisteners.push(...all));

    tick = setInterval(() => {
      if (startMs !== null && item.state === "running") {
        elapsed = (Date.now() - startMs) / 1000;
      }
    }, 500);
  });

  onDestroy(() => {
    for (const u of unlisteners) u();
    if (tick) clearInterval(tick);
  });

  function copyLogs(): void {
    const text = logs
      .map((l) => `[${l.at}] ${l.step} ${l.pct}%${l.detail ? " · " + l.detail : ""}`)
      .join("\n");
    void navigator.clipboard.writeText(text);
  }
</script>

<section class="details" aria-label={t("queue.detailsAria")}>
  <div class="metrics">
    <div class="metric">
      <span class="k">{t("queue.detailsElapsed")}</span>
      <span class="v">{fmtClock(elapsed)}</span>
    </div>
    <div class="metric">
      <span class="k">{t("queue.detailsSpeed")}</span>
      <span class="v">
        {#if pctPerSec > 0}
          {pctPerSec.toFixed(1)} %/s
        {:else}
          —
        {/if}
      </span>
    </div>
    <div class="metric">
      <span class="k">{t("queue.detailsEta")}</span>
      <span class="v">
        {#if etaSecs !== null && etaSecs < 3600 * 5}
          ~ {fmtClock(etaSecs)}
        {:else}
          —
        {/if}
      </span>
    </div>
    {#if batchInfo}
      <div class="metric">
        <span class="k">{t("queue.detailsThroughput")}</span>
        <span class="v">
          {batchInfo.done}/{batchInfo.total} lotes
          {#if lotesPerSec > 0}· {lotesPerSec.toFixed(2)} lotes/s{/if}
        </span>
      </div>
    {/if}
    {#if realtimeFactor !== null}
      <div class="metric">
        <span class="k">realtime</span>
        <span class="v">{realtimeFactor.toFixed(1)}×</span>
      </div>
    {/if}
    {#if item.summary}
      <div class="metric">
        <span class="k">{t("queue.detailsDuration")}</span>
        <span class="v">{fmtClock(item.summary.duration_secs)}</span>
      </div>
    {/if}
  </div>

  <div class="term-head">
    <span class="term-title">{t("queue.detailsLogTitle")}</span>
    <button type="button" class="copy" onclick={copyLogs}>{t("queue.detailsCopy")}</button>
  </div>
  <div class="term" bind:this={termEl} role="log" aria-live="polite">
    {#if logs.length === 0}
      <span class="empty">{t("queue.detailsEmpty")}</span>
    {:else}
      {#each logs as line, i (i)}
        <div class="line">
          <span class="at">[{line.at}]</span>
          <span class="step">{line.step}</span>
          <span class="pct">{line.pct}%</span>
          {#if line.detail}<span class="detail">· {line.detail}</span>{/if}
        </div>
      {/each}
    {/if}
  </div>
  <p class="hint">{t("queue.detailsHint")}</p>
</section>

<style>
  .details {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-2);
  }

  .metrics {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .metric {
    display: flex;
    gap: var(--space-2);
    align-items: baseline;
    font-size: var(--font-size-sm);
  }

  .k {
    color: var(--text-muted);
  }

  .v {
    font-variant-numeric: tabular-nums;
    font-weight: var(--font-weight-semibold);
  }

  .term-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .term-title {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    color: var(--text-muted);
  }

  .copy {
    font: inherit;
    font-size: var(--font-size-sm);
    cursor: pointer;
    padding: 2px var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
  }

  .copy:hover {
    border-color: var(--accent);
  }

  .term {
    max-height: 220px;
    overflow-y: auto;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: #0f1115;
    color: #c8d0dc;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12px;
    line-height: 1.5;
  }

  .line {
    display: flex;
    gap: var(--space-2);
    white-space: pre-wrap;
    word-break: break-all;
  }

  .at {
    color: #7a869a;
    flex-shrink: 0;
  }

  .step {
    color: #8ec07c;
  }

  .pct {
    color: #83a598;
    flex-shrink: 0;
  }

  .detail {
    color: #a0aec0;
  }

  .empty {
    color: var(--text-muted);
    font-family: inherit;
  }

  .hint {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--text-muted);
  }
</style>
