<script lang="ts">
  // Editor rápido de legendas (tarefa 4.6): grade editável de segmentos com
  // texto (validação ao vivo de ≤2 linhas / ≤42 chars) e timestamps in/out,
  // sincronizada com o vídeo (clique na linha → seek; reprodução → linha
  // destacada). Rolagem virtual para volumes grandes (~700-1000 linhas). Salvar
  // via `save_subtitles` (backend valida e gera SRT).

  import { invoke } from "@tauri-apps/api/core";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import TimingField from "./TimingField.svelte";
  import { showError } from "../common/ErrorToast.svelte";
  import { errMsg } from "../../lib/errors";
  import { t } from "../../lib/t";
  import { hotkeyDispatcher, type Hotkey } from "../../lib/hotkeys";

  interface Cue {
    start_ms: number;
    end_ms: number;
    text: string;
  }

  const MAX_LINES = 2;
  const MAX_CHARS = 42;
  const ROW = 68;
  const BUFFER = 5;

  let { videoPath, srtPath }: { videoPath: string; srtPath: string } = $props();

  let cues = $state<Cue[]>([]);
  let current = $state(-1);
  let selected = $state(-1);
  let showHelp = $state(false);
  let loadError = $state<string | null>(null);
  let status = $state<string | null>(null);
  let saveError = $state<string | null>(null);
  let videoEl: HTMLVideoElement | undefined = $state();
  let gridEl: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let gridH = $state(400);
  const textareas: Record<number, HTMLTextAreaElement> = {};

  const videoUrl = $derived(convertFileSrc(videoPath));
  const fileName = $derived(srtPath.split("/").pop() ?? srtPath);

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
      const toMs = (h: string, min: string, s: string, ms: string) =>
        (+h * 3600 + +min * 60 + +s) * 1000 + +ms.padEnd(3, "0");
      const text = lines
        .slice(lines.indexOf(timing) + 1)
        .join("\n")
        .trim();
      if (!text) continue;
      out.push({ start_ms: toMs(sh, sm, ss, sms), end_ms: toMs(eh, em, es, ems), text });
    }
    return out;
  }

  function rowIssues(c: Cue, idx: number): string[] {
    const issues: string[] = [];
    const lines = c.text.split("\n");
    if (lines.length > MAX_LINES) issues.push(t("editor.issueLines"));
    for (const l of lines) if (l.length > MAX_CHARS) issues.push(t("editor.issueLongLine"));
    if (c.end_ms <= c.start_ms) issues.push(t("editor.issueEnd"));
    if (idx > 0 && c.start_ms < cues[idx - 1].end_ms) issues.push(t("editor.issueOverlap"));
    return issues;
  }

  const issues = $derived(cues.map(rowIssues));
  const allValid = $derived(issues.every((a) => a.length === 0));

  // Rolagem virtual: apenas as linhas visíveis são renderizadas.
  const startIdx = $derived(Math.max(0, Math.floor(scrollTop / ROW) - BUFFER));
  const endIdx = $derived(Math.min(cues.length, startIdx + Math.ceil(gridH / ROW) + 2 * BUFFER));
  const visibleIdx = $derived(
    Array.from({ length: Math.max(0, endIdx - startIdx) }, (_, k) => startIdx + k),
  );
  const topPad = $derived(startIdx * ROW);
  const bottomPad = $derived(Math.max(0, (cues.length - endIdx) * ROW));

  function onScroll(): void {
    if (gridEl) scrollTop = gridEl.scrollTop;
  }

  function seekTo(i: number): void {
    if (videoEl) videoEl.currentTime = cues[i].start_ms / 1000;
  }

  function onTime(): void {
    if (!videoEl) return;
    const t = videoEl.currentTime * 1000;
    current = cues.findIndex((c) => t >= c.start_ms && t < c.end_ms);
  }

  // Rolagem para manter o índice visível na janela virtual.
  function scrollToIndex(i: number): void {
    if (!gridEl) return;
    const top = i * ROW;
    const bottom = top + ROW;
    const view = gridEl.scrollTop;
    const viewH = gridEl.clientHeight;
    if (top < view || bottom > view + viewH) {
      gridEl.scrollTop = Math.max(0, top - viewH / 3);
    }
  }

  function togglePlay(): void {
    if (!videoEl) return;
    if (videoEl.paused) void videoEl.play();
    else videoEl.pause();
  }

  function moveSel(delta: number): void {
    if (!cues.length) return;
    selected = Math.min(cues.length - 1, Math.max(0, selected + delta));
    scrollToIndex(selected);
  }

  function focusTextarea(i: number): void {
    // rAF: a linha-alvo só é montada após a rolagem virtual re-renderizar.
    requestAnimationFrame(() => {
      const el = textareas[i];
      if (el) {
        el.focus();
        el.select();
      }
    });
  }

  function reformatSelected(): void {
    if (selected >= 0 && selected < cues.length) reformat(selected);
  }

  function focusNextRow(delta: number): void {
    moveSel(delta);
    focusTextarea(selected);
  }

  // Registra o textarea de cada linha visível (ref de rolagem virtual).
  function registerTextarea(node: HTMLTextAreaElement, i: number) {
    textareas[i] = node;
    return {
      destroy() {
        delete textareas[i];
      },
    };
  }

  // Mantém a linha ativa visível durante a reprodução.
  $effect(() => {
    if (current >= 0) scrollToIndex(current);
  });

  function setStart(i: number, ms: number): void {
    cues[i].start_ms = ms;
  }
  function setEnd(i: number, ms: number): void {
    cues[i].end_ms = ms;
  }

  // Clicar numa linha busca o vídeo, exceto quando o clique é num campo de
  // edição (textarea/timing/botão) — aí o clique é da edição, não do seek.
  function onRowClick(i: number, e: MouseEvent): void {
    const t = e.target as HTMLElement;
    if (t.closest("textarea, input, button")) return;
    selected = i;
    seekTo(i);
  }

  // Reaplica (aproximação do formatter 1.8): quebra em ≤2 linhas de ≤42 chars
  // em fronteira de palavra. O backend valida de verdade no save.
  function reformat(i: number): void {
    const words = cues[i].text.trim().split(/\s+/).filter(Boolean);
    if (!words.length) return;
    const lines: string[] = [];
    let cur = "";
    for (const w of words) {
      const joined = cur ? `${cur} ${w}` : w;
      if (joined.length <= MAX_CHARS) cur = joined;
      else {
        if (cur) lines.push(cur);
        cur = w;
      }
    }
    if (cur) lines.push(cur);
    if (lines.length > MAX_LINES) {
      cues[i].text = `${lines[0]}\n${lines.slice(1).join(" ")}`;
    } else {
      cues[i].text = lines.join("\n");
    }
  }

  async function save(): Promise<void> {
    if (!allValid) return;
    saveError = null;
    try {
      await invoke("save_subtitles", { path: srtPath, cues });
      status = t("editor.savedIn", { file: srtPath.split("/").pop() ?? srtPath });
    } catch (e) {
      saveError = errMsg(e, "editor.errSave");
      showError(e);
    }
  }

  // Atalhos (tarefa 5.8). Letras/setas/Espaço só disparam fora de campos de
  // texto (`skipOnInput`) para não conflitar com a digitação; Ctrl+S/Ctrl+Enter
  // são acordes e funcionam em qualquer foco.
  const hotkeys: Hotkey[] = [
    { key: " ", preventDefault: true, skipOnInput: true, handler: togglePlay },
    { key: "j", skipOnInput: true, handler: () => moveSel(1) },
    { key: "k", skipOnInput: true, handler: () => moveSel(-1) },
    { key: "Tab", preventDefault: true, skipOnInput: true, handler: () => focusNextRow(1) },
    {
      key: "Tab",
      shift: true,
      preventDefault: true,
      skipOnInput: true,
      handler: () => focusNextRow(-1),
    },
    { key: "s", ctrl: true, preventDefault: true, handler: () => void save() },
    { key: "Enter", ctrl: true, preventDefault: true, handler: reformatSelected },
    { key: "F2", skipOnInput: true, handler: () => focusTextarea(selected) },
  ];
  const onKeydown = hotkeyDispatcher(hotkeys);

  onMount(() => {
    window.addEventListener("keydown", onKeydown);
    invoke<{ srt: string }>("load_preview", { video_path: videoPath, srt_path: srtPath })
      .then((d) => {
        cues = parseSrt(d.srt);
        loadError = null;
      })
      .catch((e) => {
        loadError = errMsg(e, "editor.errLoad");
        showError(e);
      });
    return () => window.removeEventListener("keydown", onKeydown);
  });

  onDestroy(() => {
    window.removeEventListener("keydown", onKeydown);
  });
</script>

<section class="editor" aria-label={t("editor.aria")}>
  <header class="bar">
    <h2 class="file">{fileName}</h2>
    <span class="count">{t("editor.count", { count: cues.length })}</span>
    <span class="spacer"></span>
    <button type="button" onclick={() => (showHelp = !showHelp)} aria-pressed={showHelp}>
      {t("editor.shortcuts")}
    </button>
    <button type="button" class="primary" onclick={save} disabled={!allValid}>
      {t("editor.save")} <span class="hint">Ctrl+S</span>
    </button>
  </header>

  {#if showHelp}
    <div class="help" role="region" aria-label={t("editor.shortcuts")}>
      <span><kbd>Espaço</kbd> {t("editor.shortcutPlay")}</span>
      <span><kbd>J</kbd>/<kbd>K</kbd> {t("editor.shortcutNav")}</span>
      <span><kbd>Tab</kbd> {t("editor.shortcutNext")}</span>
      <span><kbd>Ctrl</kbd>+<kbd>S</kbd> {t("editor.shortcutSave")}</span>
      <span><kbd>Ctrl</kbd>+<kbd>Enter</kbd> {t("editor.shortcutReformat")}</span>
      <span><kbd>F2</kbd> {t("editor.shortcutEdit")}</span>
    </div>
  {/if}

  {#if status}<p class="ok" role="status">{status}</p>{/if}
  {#if saveError}<p class="error" role="alert">{saveError}</p>{/if}

  {#if loadError}
    <p class="error" role="alert">{loadError}</p>
  {:else}
    <div class="stage">
      <video bind:this={videoEl} src={videoUrl} controls preload="metadata" ontimeupdate={onTime}>
        <track kind="captions" label={t("preview.captionLabel")} srclang="und" src="" />
      </video>
      {#if current >= 0}
        <div class="overlay" aria-live="off">{cues[current]?.text}</div>
      {/if}
    </div>

    <div class="grid" bind:this={gridEl} bind:clientHeight={gridH} onscroll={onScroll}>
      <div class="head" aria-hidden="true">
        <span class="idx">#</span>
        <span class="timing">{t("editor.colStart")}</span>
        <span class="timing">{t("editor.colEnd")}</span>
        <span class="txt">{t("editor.colText")}</span>
        <span class="ops"></span>
      </div>
      <div style="height: {topPad}px"></div>
      {#each visibleIdx as i (i)}
        <div
          class="row"
          class:active={i === current}
          class:selected={i === selected}
          role="button"
          tabindex="0"
          onclick={(e) => onRowClick(i, e)}
          onfocus={() => (selected = i)}
          onkeydown={(e) => {
            if (e.key === "Enter") seekTo(i);
          }}
        >
          <span class="idx">{i + 1}</span>
          <span class="timing">
            <TimingField
              value={cues[i].start_ms}
              onCommit={(ms) => setStart(i, ms)}
              ariaLabel={t("editor.ariaStart", { index: i + 1 })}
            />
          </span>
          <span class="timing">
            <TimingField
              value={cues[i].end_ms}
              onCommit={(ms) => setEnd(i, ms)}
              ariaLabel={t("editor.ariaEnd", { index: i + 1 })}
            />
          </span>
          <span class="txt">
            <textarea
              bind:value={cues[i].text}
              use:registerTextarea={i}
              rows={2}
              aria-label={t("editor.ariaText", { index: i + 1 })}></textarea>
          </span>
          <span class="ops">
            {#if issues[i].length > 0}
              <span class="warn" title={issues[i].join(" · ")}>⚠</span>
            {/if}
            <button
              type="button"
              class="re"
              title={t("editor.reformat")}
              onclick={() => reformat(i)}
            >
              ↻
            </button>
          </span>
        </div>
      {/each}
      <div style="height: {bottomPad}px"></div>
    </div>
  {/if}
</section>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .file {
    margin: 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
    word-break: break-all;
  }

  .count {
    color: var(--text-muted);
    font-size: var(--font-size-sm);
  }

  .spacer {
    flex: 1;
  }

  .hint {
    font-size: 0.75em;
    opacity: 0.8;
    margin-left: var(--space-1);
  }

  .help {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-4);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-2);
    color: var(--text-muted);
    font-size: var(--font-size-sm);
  }

  kbd {
    font-family: var(--font-mono, monospace);
    font-size: 0.9em;
    padding: 0 var(--space-1);
    border: 1px solid var(--border);
    border-bottom-width: 2px;
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
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

  button:hover:not(:disabled) {
    border-color: var(--accent);
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: white;
  }

  .ok {
    color: var(--success);
    margin: 0;
  }

  .error {
    color: var(--danger);
    margin: 0;
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
    max-height: 40vh;
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

  .grid {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow-y: auto;
    height: 50vh;
  }

  .head,
  .row {
    display: grid;
    grid-template-columns: 44px 100px 100px 1fr 76px;
    align-items: center;
    gap: var(--space-1);
  }

  .head {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--surface-2);
    color: var(--text-muted);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    padding: var(--space-1) var(--space-2);
    border-bottom: 1px solid var(--border);
  }

  .row {
    height: 68px;
    box-sizing: border-box;
    padding: var(--space-1) var(--space-2);
    border-bottom: 1px solid var(--border);
    cursor: pointer;
  }

  .row:hover {
    background: var(--surface-2);
  }

  .row.active {
    background: var(--accent-soft);
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .row.selected:not(.active) {
    background: var(--surface-2);
    outline: 1px solid var(--focus-ring);
    outline-offset: -1px;
  }

  .idx {
    color: var(--text-muted);
    font-size: var(--font-size-sm);
    font-variant-numeric: tabular-nums;
  }

  .timing {
    display: flex;
  }

  .txt {
    min-width: 0;
  }

  textarea {
    width: 100%;
    box-sizing: border-box;
    font: inherit;
    font-size: var(--font-size-sm);
    line-height: 1.35;
    color: var(--text);
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    padding: var(--space-1);
    resize: none;
    height: 2.7em;
  }

  textarea:hover {
    border-color: var(--border);
  }

  textarea:focus {
    outline: 2px solid var(--focus-ring);
    border-color: var(--accent);
    background: var(--surface);
  }

  .ops {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-1);
  }

  .warn {
    color: var(--warning);
    font-size: var(--font-size-md);
    cursor: help;
  }

  .re {
    padding: 2px var(--space-1);
    font-size: var(--font-size-md);
    line-height: 1;
  }
</style>
