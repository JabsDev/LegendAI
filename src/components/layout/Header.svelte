<script lang="ts">
  import { t } from "../../lib/t";
  import { getPrefs, setTheme } from "../../lib/prefs.svelte";

  let { title }: { title: string } = $props();

  // Tema reativo ao store de preferências (4.10): o toggle persiste no backend
  // e o boot restaura o valor salvo após `loadPrefs`.
  const theme = $derived(getPrefs().theme === "dark" ? "dark" : "light");

  function toggle(): void {
    setTheme(theme === "dark" ? "light" : "dark");
  }
</script>

<header class="header">
  <h1 class="title">{title}</h1>
  <div class="status">
    <span class="dot" aria-hidden="true"></span>
    <span class="status-text">{t("common.ready")}</span>
    <button type="button" class="theme-toggle" aria-pressed={theme === "dark"} onclick={toggle}>
      {theme === "light" ? t("common.darkTheme") : t("common.lightTheme")}
    </button>
  </div>
</header>

<style>
  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-5);
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  .title {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
  }

  .status {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-muted);
    font-size: var(--font-size-sm);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--success);
  }

  .theme-toggle {
    margin-left: var(--space-2);
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
  }

  .theme-toggle:hover {
    border-color: var(--accent);
  }
</style>
