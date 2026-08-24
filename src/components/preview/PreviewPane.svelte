<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { showError } from "../common/ErrorToast.svelte";
  import { errMsg } from "../../lib/errors";
  import { t } from "../../lib/t";
  import { getPrefs, savePrefs } from "../../lib/prefs.svelte";

  interface Cue {
    start: number; // segundos
    end: number; // segundos
    text: string;
  }

  type Mode = "original" | "translated" | "both";

  let { videoPath, srtPath }: { videoPath: string; srtPath: string } = $props();

  // Modo duplo (4.5): o SRT original fica ao lado do traduzido como
  // `<nome>.original.srt` (gravado pelo pipeline quando a tradução acontece).
  const originalSrtPath = $derived(srtPath.replace(/\.[^/.]+$/, "") + ".original.srt");

  let videoUrl = $derived(convertFileSrc(videoPath));
  let cues = $state<Cue[]>([]);
  let originalCues = $state<Cue[]>([]);
  let hasOriginal = $state(false);
  let mode = $state<Mode>((getPrefs().preview_mode as Mode) || "translated");
  let current = $state(-1);
  let error = $state<string | null>(null);
  let videoEl: HTMLVideoElement | undefined = $state();
  let trackUrl = $state<string | null>(null);

  // Modo restaurado das preferências (4.10): quando `loadPrefs` resolver, o
  // valor salvo pelo usuário assume o estado local.
  $effect(() => {
    const m = getPrefs().preview_mode;
    if (m === "original" || m === "translated" || m === "both") mode = m;
  });

  function setMode(m: Mode): void {
    mode = m;
    savePrefs({ preview_mode: m });
  }

  function parseSrt(raw: string): Cue[] {
    const out: Cue[] = [];
    const blocks = raw.replace(/\r\n/g, "\n").split(/\n\s*\n/);
    for (const block of blocks) {
      const lines = block.split("\n").filter((l) => l.trim() !== "");
      const timing = lines.find((l) => l.includes("-->"));
      if (!timing) continue;
      const m = timing.match(
        /(\d{1,2}):(\d{2}):(\d{2})[,.](\d{1,3})\s*-->\s*(\d{1,2}):(\d{2}):(\d{2})[,.](\d{1,3})/,
      );
      if (!m) continue;
      const [, sh, sm, ss, sms, eh, em, es, ems] = m;
      const toSec = (h: string, min: string, s: string, ms: string) =>
        +h * 3600 + +min * 60 + +s + +ms.padEnd(3, "0") / 1000;
      const text = lines
        .slice(lines.indexOf(timing) + 1)
        .join("\n")
        .trim();
      if (!text) continue;
      out.push({ start: toSec(sh, sm, ss, sms), end: toSec(eh, em, es, ems), text });
    }
    return out;
  }

  // SRT→WebVTT é trivial: mesmo timestamp, trocando `,` por `.`.
  function toVtt(raw: string): string {
    const body = raw
      .replace(/\r\n/g, "\n")
      .replace(/(\d{2}:\d{2}:\d{2}),(\d{3})/g, "$1.$2")
      .trim();
    return `WEBVTT\n\n${body}\n`;
  }

  function onTimeUpdate(): void {
    if (!videoEl) return;
    const t = videoEl.currentTime;
    current = cues.findIndex((c) => t >= c.start && t < c.end);
  }

  function seekTo(c: Cue): void {
    if (videoEl) videoEl.currentTime = c.start;
  }

  function fmtTime(secs: number): string {
    const m = Math.floor(secs / 60);
    const s = Math.floor(secs % 60);
    return `${m}:${String(s).padStart(2, "0")}`;
  }

  // No modo "ambas": tradução em cima, original embaixo — o índice é o mesmo
  // (originais e traduções compartilham timestamps). Original ausente nesse
  // índice (listas com tamanhos distintos) → cai no texto traduzido.
  const translatedText = $derived(current >= 0 ? cues[current]?.text : "");
  const originalText = $derived(
    current >= 0 ? (originalCues[current]?.text ?? cues[current]?.text) : "",
  );

  onMount(() => {
    const translated = invoke<{ srt: string }>("load_preview", {
      video_path: videoPath,
      srt_path: srtPath,
    });
    translated
      .then((d) => {
        cues = parseSrt(d.srt);
        const vtt = new Blob([toVtt(d.srt)], { type: "text/vtt" });
        trackUrl = URL.createObjectURL(vtt);
      })
      .catch((e) => {
        error = errMsg(e, "preview.errLoad");
        showError(e);
      });

    // SRT original é opcional: falha ao carregar → modo duplo desabilitado.
    invoke<{ srt: string }>("load_preview", { video_path: videoPath, srt_path: originalSrtPath })
      .then((d) => {
        originalCues = parseSrt(d.srt);
        hasOriginal = true;
      })
      .catch(() => {
        hasOriginal = false;
      });
  });

  onDestroy(() => {
    if (trackUrl) URL.revokeObjectURL(trackUrl);
  });
</script>

<div class="preview">
  {#if hasOriginal}
    <div class="modes" role="tablist" aria-label={t("preview.modeAria")}>
      <button
        type="button"
        role="tab"
        aria-selected={mode === "original"}
        class:active={mode === "original"}
        onclick={() => setMode("original")}
      >
        {t("preview.modeOriginal")}
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={mode === "translated"}
        class:active={mode === "translated"}
        onclick={() => setMode("translated")}
      >
        {t("preview.modeTranslated")}
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={mode === "both"}
        class:active={mode === "both"}
        onclick={() => setMode("both")}
      >
        {t("preview.modeBoth")}
      </button>
    </div>
  {/if}

  <div class="stage">
    <video
      bind:this={videoEl}
      src={videoUrl}
      controls
      preload="metadata"
      ontimeupdate={onTimeUpdate}
    >
      <track kind="captions" label={t("preview.captionLabel")} srclang="und" src={trackUrl ?? ""} />
    </video>
    {#if current >= 0}
      {#if mode === "both"}
        <div class="overlay both" aria-live="off">
          <span class="line translated">{translatedText}</span>
          <span class="line original">{originalText}</span>
        </div>
      {:else if mode === "original"}
        <div class="overlay" aria-live="off">{originalText}</div>
      {:else}
        <div class="overlay" aria-live="off">{translatedText}</div>
      {/if}
    {/if}
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <ol class="cues">
    {#each cues as cue, i (cue.start + cue.text)}
      <li class:active={i === current}>
        <button type="button" onclick={() => seekTo(cue)}>
          <span class="time">{fmtTime(cue.start)}</span>
          <span class="text">
            {#if mode === "both" && hasOriginal}
              <span class="line translated">{cue.text}</span>
              <span class="line original">{originalCues[i]?.text ?? ""}</span>
            {:else}
              {cue.text}
            {/if}
          </span>
        </button>
      </li>
    {/each}
  </ol>
</div>

<style>
  .preview {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .modes {
    display: flex;
    gap: var(--space-1);
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-2);
    width: fit-content;
  }

  .modes button {
    font: inherit;
    cursor: pointer;
    border: none;
    background: transparent;
    color: var(--text-muted);
    padding: var(--space-1) var(--space-3);
    border-radius: var(--radius-sm);
  }

  .modes button:hover {
    color: var(--text);
  }

  .modes button.active {
    background: var(--accent);
    color: white;
  }

  .stage {
    position: relative;
    background: #000;
    border-radius: var(--radius);
    overflow: hidden;
  }

  video {
    display: block;
    width: 100%;
    max-height: 60vh;
    background: #000;
  }

  .overlay {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 12%;
    margin: 0 auto;
    max-width: 90%;
    padding: var(--space-1) var(--space-3);
    background: rgba(0, 0, 0, 0.75);
    color: #fff;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-medium);
    text-align: center;
    white-space: pre-line;
    border-radius: var(--radius-sm);
    pointer-events: none;
  }

  .overlay.both {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .overlay .original,
  .cues .original {
    color: #c9ccd1;
    font-weight: var(--font-weight-normal);
    font-size: 0.85em;
  }

  .error {
    color: var(--danger);
    margin: 0;
  }

  .cues {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 30vh;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: var(--space-2);
  }

  .cues button {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    width: 100%;
    text-align: left;
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    font: inherit;
  }

  .cues button:hover {
    background: var(--surface-2);
  }

  .cues li.active button {
    background: var(--accent-soft);
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .cues .text {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .time {
    flex: none;
    color: var(--text-muted);
    font-size: var(--font-size-sm);
    font-variant-numeric: tabular-nums;
    min-width: 52px;
  }

  .text {
    word-break: break-word;
  }
</style>
