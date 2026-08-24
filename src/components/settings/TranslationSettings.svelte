<script lang="ts">
  import { t } from "../../lib/t";
  import { getPrefs, savePrefs, type TranslationOptions } from "../../lib/prefs.svelte";
  import { TARGET_LANGS } from "../../lib/languages";

  // Opções avançadas de tradução (tarefa 5.4). O backend (prefs) é a fonte
  // persistida; cada mudança grava via `savePrefs` com debounce de 500ms.
  const opts = $derived(getPrefs().translation_options);
  const targetLang = $derived(getPrefs().last_language_pair?.target ?? "pt");

  function update(patch: Partial<TranslationOptions>): void {
    savePrefs({ translation_options: { ...opts, ...patch } });
  }

  function setTarget(lang: string): void {
    const src = getPrefs().last_language_pair?.source ?? "auto";
    savePrefs({ last_language_pair: { source: src, target: lang } });
  }
</script>

<section class="trans" aria-label={t("settings.translation.title")}>
  <h3>{t("settings.translation.title")}</h3>
  <p class="hint">{t("settings.translation.hint")}</p>

  <div class="field">
    <span class="label" id="formality-label">{t("settings.translation.formality")}</span>
    <div class="seg" role="radiogroup" aria-labelledby="formality-label">
      <button
        type="button"
        role="radio"
        aria-checked={opts.formality === "colloquial"}
        onclick={() => update({ formality: "colloquial" })}
      >
        {t("settings.translation.colloquial")}
      </button>
      <button
        type="button"
        role="radio"
        aria-checked={opts.formality === "formal"}
        onclick={() => update({ formality: "formal" })}
      >
        {t("settings.translation.formal")}
      </button>
    </div>
  </div>

  <div class="field col">
    <label class="label" for="custom-instructions">
      {t("settings.translation.instructions")}
    </label>
    <textarea
      id="custom-instructions"
      value={opts.custom_instructions}
      placeholder={t("settings.translation.instructionsHint")}
      oninput={(e) => update({ custom_instructions: e.currentTarget.value })}></textarea>
  </div>

  <div class="field">
    <label class="label" for="context-size">{t("settings.translation.contextSize")}</label>
    <select
      id="context-size"
      value={opts.context_size}
      onchange={(e) => update({ context_size: Number(e.currentTarget.value) })}
    >
      {#each [0, 1, 2, 3, 5] as n (n)}
        <option value={n}>{n}</option>
      {/each}
    </select>
  </div>

  <div class="field">
    <label class="label" for="target-lang">{t("settings.translation.targetLang")}</label>
    <select id="target-lang" value={targetLang} onchange={(e) => setTarget(e.currentTarget.value)}>
      {#each TARGET_LANGS as lang (lang.code)}
        <option value={lang.code}>{lang.label} ({lang.code})</option>
      {/each}
    </select>
  </div>
</section>

<style>
  .trans {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .trans h3 {
    margin: 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }

  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--font-size-sm);
  }

  .field {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .field.col {
    flex-direction: column;
    align-items: stretch;
  }

  .label {
    color: var(--text-muted);
    font-size: var(--font-size-sm);
  }

  .seg {
    display: flex;
    gap: var(--space-1);
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-2);
  }

  .seg button {
    font: inherit;
    cursor: pointer;
    border: none;
    background: transparent;
    color: var(--text-muted);
    padding: var(--space-1) var(--space-3);
    border-radius: var(--radius-sm);
  }

  .seg button:hover {
    color: var(--text);
  }

  .seg button[aria-checked="true"] {
    background: var(--accent);
    color: white;
  }

  textarea,
  select {
    font: inherit;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-2);
  }

  textarea {
    min-height: 80px;
    resize: vertical;
  }
</style>
