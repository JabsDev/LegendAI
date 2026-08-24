<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../../lib/t";
  import { loadGlossary, saveGlossary, type GlossaryEntry } from "../../lib/prefs.svelte";

  // Glossário do usuário (tarefa 5.6): termos fixos aplicados a todas as
  // traduções. Persistido no backend (`glossary.toml`) via `saveGlossary`
  // (debounced) a cada mudança.

  let entries = $state<GlossaryEntry[]>([]);
  let newTerm = $state("");
  let newTranslation = $state("");
  let newNote = $state("");

  onMount(async () => {
    entries = await loadGlossary();
  });

  function add(): void {
    const term = newTerm.trim();
    const translation = newTranslation.trim();
    if (!term || !translation) return;
    entries.push({ term, translation, note: newNote.trim() || null });
    newTerm = "";
    newTranslation = "";
    newNote = "";
    saveGlossary(entries);
  }

  function remove(i: number): void {
    entries.splice(i, 1);
    saveGlossary(entries);
  }
</script>

<section class="glossary" aria-label={t("settings.glossary.title")}>
  <h3>{t("settings.glossary.title")}</h3>
  <p class="hint">{t("settings.glossary.hint")}</p>

  <div class="rows">
    {#each entries as entry, i (entry.term)}
      <div class="row">
        <label class="label">
          <span>{t("settings.glossary.term")}</span>
          <input
            type="text"
            value={entry.term}
            oninput={(e) => {
              entry.term = e.currentTarget.value;
              saveGlossary(entries);
            }}
          />
        </label>
        <label class="label">
          <span>{t("settings.glossary.translation")}</span>
          <input
            type="text"
            value={entry.translation}
            oninput={(e) => {
              entry.translation = e.currentTarget.value;
              saveGlossary(entries);
            }}
          />
        </label>
        <label class="label note">
          <span>{t("settings.glossary.note")}</span>
          <input
            type="text"
            placeholder={t("settings.glossary.noteHint")}
            value={entry.note ?? ""}
            oninput={(e) => {
              entry.note = e.currentTarget.value.trim() || null;
              saveGlossary(entries);
            }}
          />
        </label>
        <button
          type="button"
          class="remove"
          aria-label={t("settings.glossary.remove")}
          onclick={() => remove(i)}>✕</button
        >
      </div>
    {/each}
  </div>

  {#if entries.length === 0}
    <p class="empty">{t("settings.glossary.empty")}</p>
  {/if}

  <div class="add">
    <label class="label">
      <span>{t("settings.glossary.term")}</span>
      <input type="text" bind:value={newTerm} />
    </label>
    <label class="label">
      <span>{t("settings.glossary.translation")}</span>
      <input type="text" bind:value={newTranslation} />
    </label>
    <button type="button" class="add-btn" onclick={add}>
      {t("settings.glossary.add")}
    </button>
  </div>
</section>

<style>
  .glossary {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .glossary h3 {
    margin: 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }

  .hint,
  .empty {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--font-size-sm);
  }

  .rows {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .row {
    display: flex;
    align-items: end;
    gap: var(--space-2);
  }

  .label {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--text-muted);
    font-size: var(--font-size-sm);
  }

  .row .label:nth-child(1),
  .row .label:nth-child(2) {
    flex: 1;
  }

  .row .label.note {
    flex: 1.5;
  }

  input {
    font: inherit;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2);
  }

  .remove {
    font: inherit;
    cursor: pointer;
    border: none;
    background: transparent;
    color: var(--danger);
    padding: var(--space-1);
    line-height: 1;
  }

  .remove:hover {
    color: var(--text);
  }

  .add {
    display: flex;
    align-items: end;
    gap: var(--space-2);
    padding-top: var(--space-2);
    border-top: 1px solid var(--border);
  }

  .add .label {
    flex: 1;
  }

  .add-btn {
    font: inherit;
    cursor: pointer;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: white;
    padding: var(--space-1) var(--space-3);
  }
</style>
