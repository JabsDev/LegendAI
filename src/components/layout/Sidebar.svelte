<script lang="ts">
  import { t } from "../../lib/t";

  type Route = "import" | "queue" | "models" | "settings";

  let { active, onNavigate }: { active: Route; onNavigate: (route: Route) => void } = $props();

  const items = $derived([
    { id: "import", label: t("app.import") },
    { id: "queue", label: t("queue.title") },
    { id: "models", label: t("app.models") },
    { id: "settings", label: t("app.settings") },
  ] as { id: Route; label: string }[]);
</script>

<aside class="sidebar">
  <div class="brand">
    <span class="brand-name">LegendAI</span>
  </div>
  <nav aria-label={t("nav.main")}>
    <ul>
      {#each items as item (item.id)}
        <li>
          <button
            type="button"
            class="nav-item"
            class:active={active === item.id}
            aria-current={active === item.id ? "page" : undefined}
            onclick={() => onNavigate(item.id)}
          >
            {item.label}
          </button>
        </li>
      {/each}
    </ul>
  </nav>
</aside>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    width: var(--sidebar-width);
    flex-shrink: 0;
    background: var(--sidebar-bg);
    border-right: 1px solid var(--border);
    padding: var(--space-4);
  }

  .brand {
    padding: var(--space-2) var(--space-2) var(--space-5);
  }

  .brand-name {
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    color: var(--text);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .nav-item {
    display: block;
    width: 100%;
    text-align: left;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-muted);
    font: inherit;
    cursor: pointer;
  }

  .nav-item:hover {
    color: var(--text);
    background: var(--surface-2);
  }

  .nav-item.active {
    color: var(--text);
    background: var(--accent-soft);
    font-weight: var(--font-weight-semibold);
  }
</style>
