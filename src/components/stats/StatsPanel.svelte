<script lang="ts">
  import { t } from "../../lib/t";

  export interface JobStats {
    processing_secs: number;
    duration_secs: number;
    segments: number;
    avg_cps: number;
    speech_coverage_pct: number;
    translation_ratio: number;
    tier: string;
    goal_processing_secs: number;
  }

  let { stats }: { stats: JobStats } = $props();

  // Fração do orçamento (meta do tier) consumida pelo tempo real: ≤100% = dentro
  // da meta (a barra enche à medida que o processamento gasta a meta).
  const budgetPct = $derived(
    stats.goal_processing_secs > 0 && stats.processing_secs > 0
      ? Math.min(100, (stats.processing_secs / stats.goal_processing_secs) * 100)
      : 0,
  );
  const onTarget = $derived(
    stats.goal_processing_secs > 0 && stats.processing_secs <= stats.goal_processing_secs,
  );
  const tierNumber = $derived(stats.tier.replace(/^tier/i, ""));

  function fmtTime(secs: number): string {
    if (!secs) return "—";
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = Math.floor(secs % 60);
    const mm = String(m).padStart(2, "0");
    const ss = String(s).padStart(2, "0");
    return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
  }

  function fmtNum(v: number): string {
    return v ? v.toFixed(1) : "—";
  }
</script>

<section class="stats" aria-label={t("stats.aria")}>
  <h4>{t("stats.title")}</h4>
  <dl>
    <div>
      <dt>{t("stats.processingTime")}</dt>
      <dd>{fmtTime(stats.processing_secs)}</dd>
    </div>
    <div>
      <dt>{t("stats.videoDuration")}</dt>
      <dd>{fmtTime(stats.duration_secs)}</dd>
    </div>
    <div>
      <dt>{t("stats.subtitles")}</dt>
      <dd>{stats.segments}</dd>
    </div>
    <div>
      <dt>{t("stats.avgCps")}</dt>
      <dd>{fmtNum(stats.avg_cps)}</dd>
    </div>
    <div>
      <dt>{t("stats.speechCoverage")}</dt>
      <dd>{fmtNum(stats.speech_coverage_pct)}%</dd>
    </div>
    <div>
      <dt>{t("stats.translationRate")}</dt>
      <dd>{fmtNum(stats.translation_ratio)}×</dd>
    </div>
  </dl>

  <div class="goal">
    <div class="goal-head">
      <span class="goal-label">{t("stats.tierGoal", { tier: tierNumber })}</span>
      <span class="goal-times">
        {t("stats.realVsGoal", {
          real: fmtTime(stats.processing_secs),
          goal: fmtTime(stats.goal_processing_secs),
        })}
      </span>
      <span class:ok={onTarget} class:over={!onTarget} class="goal-status" role="status">
        {onTarget ? t("stats.onTarget") : t("stats.overTarget")}
      </span>
    </div>
    <div
      class="bar"
      role="progressbar"
      aria-valuenow={Math.round(budgetPct)}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <div class="fill" class:over={!onTarget} style="width: {budgetPct}%"></div>
    </div>
  </div>
</section>

<style>
  .stats {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-2);
  }

  .stats h4 {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    color: var(--text-muted);
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
    min-width: 160px;
    font-size: var(--font-size-sm);
  }

  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }

  .goal {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .goal-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    font-size: var(--font-size-sm);
  }

  .goal-label {
    font-weight: var(--font-weight-semibold);
  }

  .goal-times {
    color: var(--text-muted);
  }

  .goal-status {
    margin-left: auto;
    white-space: nowrap;
  }

  .goal-status.ok {
    color: var(--success);
  }

  .goal-status.over {
    color: var(--warning);
  }

  .bar {
    height: 10px;
    border-radius: 5px;
    background: var(--surface);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--success);
    transition: width 0.2s;
  }

  .fill.over {
    background: var(--warning);
  }
</style>
