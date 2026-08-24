<script module lang="ts">
  // Estado global compartilhado (tarefa 4.8): qualquer componente chama
  // `showError(e)` e o toast/dialog é exibido pelo <ErrorToast> montado no
  // App. `queue`/`logPath` vivem no escopo do módulo — a instância no App
  // assina a reatividade (Svelte 5 runes em <script module>).
  import { toToast, type ErrorToastInfo } from "../../lib/errors";
  import { invoke } from "@tauri-apps/api/core";

  const TOAST_MS = 6000;

  let queue = $state<ErrorToastInfo[]>([]);
  // null = ainda não resolvido; "" = indisponível.
  let logPath = $state<string | null>(null);

  // Centraliza a captura de erros de comandos IPC: resolve o log path (uma
  // vez) e enfileira o toast. Erros "inesperados" abrem o dialog (não são
  // auto-dispensados) e ficam até o usuário fechar.
  export function showError(e: unknown): void {
    if (logPath === null) {
      void invoke<{ log_path: string }>("get_app_info")
        .then((info) => (logPath = info.log_path || ""))
        .catch(() => (logPath = ""));
    }
    const info = toToast(e, logPath ?? "");
    queue = [...queue, info];
    if (!info.unexpected) {
      setTimeout(() => dismiss(info.id), TOAST_MS);
    }
  }

  export function dismiss(id: number): void {
    queue = queue.filter((item) => item.id !== id);
  }
</script>

<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { GITHUB_ISSUES_BASE } from "../../lib/errors";
  import { t } from "../../lib/t";

  let { onNavigate }: { onNavigate: (route: string) => void } = $props();

  const unexpected = $derived(queue.find((item) => item.unexpected) ?? null);

  function actionFor(item: ErrorToastInfo): void {
    if (item.action) onNavigate(item.action.route);
    dismiss(item.id);
  }

  async function openIssue(): Promise<void> {
    const params = new URLSearchParams({
      title: t("errors.issueTitle"),
      body: `${t("errors.unexpected.message")}\n\n${t("errors.logPath")}: ${
        logPath || t("errors.logUnavailable")
      }`,
    });
    await openUrl(`${GITHUB_ISSUES_BASE}?${params}`);
  }

  async function copyLogPath(): Promise<void> {
    const value = logPath || t("errors.logUnavailable");
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      /* clipboard indisponível — sem ação */
    }
  }
</script>

{#if unexpected}
  <div class="overlay" role="alertdialog" aria-modal="true" aria-labelledby="err-dialog-title">
    <div class="dialog">
      <h2 id="err-dialog-title">{unexpected.title}</h2>
      <p class="msg">{unexpected.message}</p>
      {#if unexpected.hint}
        <p class="hint">{unexpected.hint}</p>
      {/if}
      <p class="log">
        <span class="log-label">{t("errors.logPath")}</span>
        <code class="log-value">{logPath || t("errors.logUnavailable")}</code>
      </p>
      <div class="actions">
        <button type="button" onclick={copyLogPath}>{t("errors.copyPath")}</button>
        <button type="button" class="primary" onclick={openIssue}>{t("errors.report")}</button>
        <button type="button" onclick={() => dismiss(unexpected.id)}>{t("errors.close")}</button>
      </div>
    </div>
  </div>
{:else}
  <div class="toasts" aria-live="polite">
    {#each queue as item (item.id)}
      <div class="toast toast-{item.severity}" role="alert">
        <div class="body">
          <strong class="title">{item.title}</strong>
          <span class="msg">{item.message}</span>
          {#if item.hint}
            <span class="hint">{item.hint}</span>
          {/if}
        </div>
        <div class="toast-actions">
          {#if item.action}
            <button type="button" onclick={() => actionFor(item)}>{t(item.action.label)}</button>
          {/if}
          <button
            type="button"
            class="close"
            aria-label={t("errors.dismiss")}
            onclick={() => dismiss(item.id)}
          >
            ×
          </button>
        </div>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toasts {
    position: fixed;
    top: var(--space-4);
    right: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    z-index: 1000;
    max-width: 380px;
  }

  .toast {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-3) var(--space-3) var(--space-4);
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--border);
    border-left-width: 4px;
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.18);
  }

  .toast-error {
    border-left-color: var(--danger);
  }

  .toast-warning {
    border-left-color: var(--warning);
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .title {
    font-weight: var(--font-weight-semibold);
  }

  .msg {
    font-size: var(--font-size-sm);
    color: var(--text-muted);
  }

  .hint {
    font-size: var(--font-size-sm);
    color: var(--text-muted);
    opacity: 0.85;
  }

  .toast-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
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

  button:hover {
    border-color: var(--accent);
  }

  .close {
    padding: var(--space-1) var(--space-2);
    line-height: 1;
    opacity: 0.7;
  }

  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: white;
  }

  .overlay {
    position: fixed;
    inset: 0;
    z-index: 1100;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgb(0 0 0 / 0.45);
  }

  .dialog {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 460px;
    width: calc(100% - var(--space-8));
    padding: var(--space-5);
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--border);
  }

  .dialog h2 {
    margin: 0;
    font-size: var(--font-size-lg);
    color: var(--danger);
  }

  .msg {
    margin: 0;
  }

  .hint {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--text-muted);
  }

  .log {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
  }

  .log-label {
    font-size: var(--font-size-sm);
    color: var(--text-muted);
  }

  .log-value {
    word-break: break-all;
    font-size: var(--font-size-sm);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
</style>
