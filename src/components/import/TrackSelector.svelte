<script lang="ts">
  import type { VideoInspection } from "./ImportDropzone.svelte";
  import { t } from "../../lib/t";
  import { TARGET_LANGS } from "../../lib/languages";
  import { getPrefs } from "../../lib/prefs.svelte";

  type Source = "embedded" | "audio";

  let {
    inspection,
    onTranscribe,
    onUseEmbedded,
  }: {
    inspection: VideoInspection;
    onTranscribe: (
      trackIndex: number,
      opts: { translate: boolean; targetLang: string; sourceLang: string },
    ) => void;
    onUseEmbedded: (streamIndex: number, opts: { translate: boolean; targetLang: string }) => void;
  } = $props();

  const hasAudio = $derived(inspection.audio_tracks.length > 0);
  const hasEmbedded = $derived(inspection.subtitle_streams.length > 0);

  let source = $state<Source>("audio");
  let audioIndex = $state<number>(0);
  let subIndex = $state<number>(0);
  let translate = $state(true);
  let targetLang = $state(getPrefs().last_language_pair?.target ?? "pt");
  let sourceLang = $state("auto");

  $effect(() => {
    audioIndex = inspection.audio_tracks[0]?.index ?? 0;
    subIndex = inspection.subtitle_streams[0]?.index ?? 0;
    source = hasEmbedded ? "embedded" : "audio";
    targetLang = getPrefs().last_language_pair?.target ?? targetLang;
  });

  const ISO3_TO_2: Record<string, string> = {
    eng: "en",
    jpn: "ja",
    por: "pt",
    spa: "es",
    fra: "fr",
    deu: "de",
    ita: "it",
    zho: "zh",
    chi: "zh",
    ara: "ar",
    rus: "ru",
    kor: "ko",
    hin: "hi",
    nld: "nl",
    pol: "pl",
    tur: "tr",
    vie: "vi",
    tha: "th",
    ind: "id",
    yue: "yue",
  };

  function normalizeLang(code: string | null): string | null {
    if (!code) return null;
    const c = code.trim().toLowerCase();
    if (c === "und" || c === "") return null;
    return ISO3_TO_2[c] ?? c;
  }

  function langLabel(lang: string | null): string {
    const n = normalizeLang(lang);
    return n ? n.toUpperCase() : t("track.unknownLang");
  }

  function fmtDuration(secs: number): string {
    if (!secs) return t("track.unknownDuration");
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = Math.floor(secs % 60);
    const mm = String(m).padStart(2, "0");
    const ss = String(s).padStart(2, "0");
    return h > 0 ? `${h}:${mm}:${ss}` : `${m}:${ss}`;
  }

  function proceed(): void {
    const opts = { translate, targetLang, sourceLang };
    if (source === "embedded" && hasEmbedded) onUseEmbedded(subIndex, opts);
    else if (source === "audio" && hasAudio) onTranscribe(audioIndex, opts);
  }
</script>

<section class="track-selector" aria-label={t("track.aria")}>
  <div class="meta">
    <span class="name">{inspection.file_name}</span>
    <span class="duration">{fmtDuration(inspection.duration_secs)}</span>
  </div>

  <fieldset class="group" disabled={!hasEmbedded}>
    <legend>{t("track.embedded")}</legend>
    {#if hasEmbedded}
      {#each inspection.subtitle_streams as sub (sub.index)}
        <label class="option">
          <input
            type="radio"
            name="source"
            checked={source === "embedded" && subIndex === sub.index}
            onchange={() => {
              source = "embedded";
              subIndex = sub.index;
            }}
          />
          <span>
            {t("track.subtitle", { codec: sub.codec })}
            {#if sub.lang}<span class="tag">({langLabel(sub.lang)})</span>{/if}
            {#if sub.default}<span class="tag">{t("track.default")}</span>{/if}
          </span>
        </label>
      {/each}
    {:else}
      <p class="muted">{t("track.embeddedNone")}</p>
    {/if}
  </fieldset>

  <fieldset class="group">
    <legend>{t("track.transcribe")}</legend>
    {#if hasAudio}
      {#each inspection.audio_tracks as track (track.index)}
        <label class="option">
          <input
            type="radio"
            name="source"
            checked={source === "audio" && audioIndex === track.index}
            onchange={() => {
              source = "audio";
              audioIndex = track.index;
            }}
          />
          <span>
            {t("track.audioTrack", { index: track.index })}
            {#if track.lang}<span class="tag">({langLabel(track.lang)})</span>{/if}
            {#if track.channels > 0}
              <span class="tag">{t("track.channels", { count: track.channels })}</span>
            {/if}
            {#if track.default}<span class="tag">{t("track.default")}</span>{/if}
          </span>
        </label>
      {/each}
      <label class="field" style="margin-top: 8px;">
        <span class="label">{t("track.sourceLang")}</span>
        <select bind:value={sourceLang} disabled={source !== "audio"}>
          <option value="auto">{t("track.sourceAuto")} (auto)</option>
          {#each TARGET_LANGS as lang (lang.code)}
            <option value={lang.code}>{lang.label} ({lang.code})</option>
          {/each}
        </select>
      </label>
      {#if inspection.audio_tracks.some((t) => (t.lang ?? "").toLowerCase() === "eng" || (t.lang ?? "").toLowerCase() === "en")}
        <p class="muted" style="font-size: 12px;">{t("track.sourceHint")}</p>
      {/if}
    {:else}
      <p class="muted warn">{t("track.noAudio")}</p>
    {/if}
  </fieldset>

  <fieldset class="group">
    <legend>{t("track.translateOpts")}</legend>
    <label class="option">
      <input type="checkbox" bind:checked={translate} />
      <span>{t("track.doTranslate")}</span>
    </label>
    <label class="field">
      <span class="label">{t("track.targetLang")}</span>
      <select bind:value={targetLang} disabled={!translate}>
        {#each TARGET_LANGS as lang (lang.code)}
          <option value={lang.code}>{lang.label} ({lang.code})</option>
        {/each}
      </select>
    </label>
  </fieldset>

  <button
    type="button"
    class="continue"
    disabled={!(source === "audio" && hasAudio) && !(source === "embedded" && hasEmbedded)}
    onclick={proceed}
  >
    {t("track.continue")}
  </button>
</section>

<style>
  .track-selector {
    display: flex;
    flex-direction: column;
    gap: 16px;
    max-width: 640px;
  }

  .meta {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  .name {
    font-weight: 600;
    word-break: break-all;
  }

  .duration {
    color: var(--text-muted);
    font-size: var(--font-size-sm);
    white-space: nowrap;
  }

  .group {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  legend {
    padding: 0 4px;
    font-size: var(--font-size-sm);
    opacity: 0.7;
  }

  .option {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }

  .tag {
    opacity: 0.6;
    font-size: var(--font-size-sm);
  }

  .muted {
    opacity: 0.6;
    font-size: var(--font-size-sm);
    margin: 0;
  }

  .warn {
    color: var(--warning);
    opacity: 1;
  }

  .continue {
    align-self: flex-start;
    padding: 6px 16px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
    font: inherit;
  }

  .continue:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .continue:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .field {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .field .label {
    color: var(--text-muted);
    font-size: var(--font-size-sm);
  }

  .field select {
    font: inherit;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 4px 8px;
  }

  .field select:disabled {
    opacity: 0.5;
  }
</style>
