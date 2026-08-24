<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import Sidebar from "./components/layout/Sidebar.svelte";
  import Header from "./components/layout/Header.svelte";
  import ModelList from "./components/models/ModelList.svelte";
  import FirstRun from "./components/onboarding/FirstRun.svelte";
  import ImportDropzone, { type VideoInspection } from "./components/import/ImportDropzone.svelte";
  import TrackSelector from "./components/import/TrackSelector.svelte";
  import QueueView from "./components/queue/QueueView.svelte";
  import TranslationSettings from "./components/settings/TranslationSettings.svelte";
  import GlossaryEditor from "./components/settings/GlossaryEditor.svelte";
  import ErrorToast, { showError } from "./components/common/ErrorToast.svelte";
  import { t } from "./lib/t";
  import { getLang, setLang } from "./i18n/index.svelte";
  import { getPrefs, loadPrefs, savePrefs, setTheme } from "./lib/prefs.svelte";

  type Route = "import" | "queue" | "models" | "settings";

  interface PipelineSource {
    type: "audio" | "embedded";
    track_index?: number;
    stream_index?: number;
  }

  const TITLES = $derived<Record<Route, string>>({
    import: t("app.import"),
    queue: t("queue.title"),
    models: t("app.models"),
    settings: t("app.settings"),
  });

  let route = $state<Route>("import");
  let inspection = $state<VideoInspection | null>(null);
  let firstRun = $state(false);

  // Restaura as últimas escolhas persistidas (4.10) após o boot: tema e idioma
  // salvos no backend voltam a valer (o localStorage cobre o render inicial).
  onMount(() => {
    // Primeira execução (config ausente, 6.4) → tela de boas-vindas com tier e
    // download dos modelos recomendados.
    void invoke<{ first_run: boolean }>("get_onboarding")
      .then((o) => (firstRun = o.first_run))
      .catch(() => {});
    void loadPrefs().then(() => {
      const p = getPrefs();
      if (p.ui_language === "pt" || p.ui_language === "en") setLang(p.ui_language);
      if (p.theme === "light" || p.theme === "dark") setTheme(p.theme);
      // Config antiga com `theme = system`: persiste o tema real em uso.
      else if (document.documentElement.dataset.theme === "dark") setTheme("dark");
    });
  });

  // Arquivos recentes persistidos (4.10) — reabrem direto na importação.
  const recents = $derived(getPrefs().recent_files);

  function recentName(p: string): string {
    return p.split(/[\\/]/).pop() || p;
  }

  async function reopen(path: string): Promise<void> {
    try {
      const insp = await invoke<VideoInspection>("inspect_video", { path });
      inspection = insp;
    } catch (e) {
      showError(e);
    }
  }

  // Adiciona o vídeo à fila de processamento (4.9) e abre a tela da fila.
  async function enqueue(
    source: PipelineSource,
    opts: { translate: boolean; targetLang: string; sourceLang?: string },
  ): Promise<void> {
    if (!inspection) return;
    try {
      // Persiste o par escolhido (fonte da próxima importação e default de Settings)
      savePrefs({
        recent_file: inspection.path,
        last_language_pair: { source: opts.sourceLang ?? "auto", target: opts.targetLang },
      });
      await invoke("queue_enqueue", {
        input_path: inspection.path,
        source,
        options: {
          translate: opts.translate,
          target_lang: opts.targetLang,
          source_lang: opts.sourceLang ?? "auto",
        },
      });
      route = "queue";
    } catch (e) {
      showError(e);
    }
  }

  function handleTranscribe(
    trackIndex: number,
    opts: { translate: boolean; targetLang: string; sourceLang: string },
  ): void {
    void enqueue({ type: "audio", track_index: trackIndex }, opts);
  }

  function handleUseEmbedded(
    streamIndex: number,
    opts: { translate: boolean; targetLang: string },
  ): void {
    void enqueue({ type: "embedded", stream_index: streamIndex }, opts);
  }
</script>

<div class="app">
  {#if firstRun}
    <FirstRun done={() => (firstRun = false)} />
  {:else}
    <Sidebar active={route} onNavigate={(r) => (route = r)} />
    <ErrorToast onNavigate={(r) => (route = r as Route)} />
    <div class="main">
      <Header title={TITLES[route]} />
      <main class="content">
        {#if route === "import"}
          <div class="import">
            <ImportDropzone onInspect={(i) => (inspection = i)} />
            {#if inspection}
              {#key inspection.file_name}
                <TrackSelector
                  {inspection}
                  onTranscribe={handleTranscribe}
                  onUseEmbedded={handleUseEmbedded}
                />
              {/key}
            {/if}
            {#if recents.length > 0}
              <section class="recents" aria-label={t("import.recents")}>
                <h3>{t("import.recents")}</h3>
                <ul>
                  {#each recents as path (path)}
                    <li>
                      <button type="button" onclick={() => void reopen(path)}>
                        {recentName(path)}
                      </button>
                    </li>
                  {/each}
                </ul>
              </section>
            {/if}
          </div>
        {:else if route === "queue"}
          <QueueView />
        {:else if route === "models"}
          <ModelList />
        {:else}
          <section class="settings" aria-label={TITLES[route]}>
            <h2>{t("app.settings")}</h2>
            <TranslationSettings />
            <GlossaryEditor />
            <div class="field">
              <span class="label" id="lang-label">{t("settings.language")}</span>
              <div class="langs" role="group" aria-labelledby="lang-label">
                <button
                  type="button"
                  aria-pressed={getLang() === "pt"}
                  onclick={() => setLang("pt")}
                >
                  {t("settings.langPt")}
                </button>
                <button
                  type="button"
                  aria-pressed={getLang() === "en"}
                  onclick={() => setLang("en")}
                >
                  {t("settings.langEn")}
                </button>
              </div>
            </div>
          </section>
        {/if}
      </main>
    </div>
  {/if}
</div>

<style>
  .app {
    display: flex;
    height: 100vh;
  }

  .main {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }

  .content {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-5);
  }

  .import {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .settings {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 560px;
  }

  .settings h2 {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
  }

  .field {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .label {
    color: var(--text-muted);
    font-size: var(--font-size-sm);
  }

  .langs {
    display: flex;
    gap: var(--space-1);
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-2);
  }

  .langs button {
    font: inherit;
    cursor: pointer;
    border: none;
    background: transparent;
    color: var(--text-muted);
    padding: var(--space-1) var(--space-3);
    border-radius: var(--radius-sm);
  }

  .langs button:hover {
    color: var(--text);
  }

  .langs button[aria-pressed="true"] {
    background: var(--accent);
    color: white;
  }

  .recents {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: 640px;
  }

  .recents h3 {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    color: var(--text-muted);
  }

  .recents ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .recents button {
    font: inherit;
    cursor: pointer;
    text-align: left;
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
    word-break: break-all;
  }

  .recents button:hover {
    border-color: var(--accent);
  }
</style>
