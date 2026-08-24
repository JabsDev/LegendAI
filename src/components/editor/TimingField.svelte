<script lang="ts">
  // Campo de edição de timestamp no formato SRT (HH:MM:SS,mmm). Mostra o valor
  // formatado; ao perder o foco ou Enter, parseia e chama `onCommit(ms)` se
  // válido (estado de erro visual enquanto inválido). Escape restaura o valor
  // original. O texto é ressincronizado com `value` apenas quando o campo não
  // está em foco (não clobber na digitação).

  let {
    value,
    onCommit,
    ariaLabel,
  }: {
    value: number; // ms
    onCommit: (ms: number) => void;
    ariaLabel?: string;
  } = $props();

  function fmt(ms: number): string {
    const total = Math.max(0, Math.floor(ms));
    const h = Math.floor(total / 3600000);
    const m = Math.floor((total % 3600000) / 60000);
    const s = Math.floor((total % 60000) / 1000);
    const mm = total % 1000;
    return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(s).padStart(
      2,
      "0",
    )},${String(mm).padStart(3, "0")}`;
  }

  function parse(input: string): number | null {
    const m = input.trim().match(/^(\d{1,2}):(\d{2}):(\d{2})[,.](\d{1,3})$/);
    if (!m) return null;
    const [, h, mm, s, ms] = m;
    if (+mm >= 60 || +s >= 60) return null;
    return +h * 3600000 + +mm * 60000 + +s * 1000 + +ms.padEnd(3, "0");
  }

  // O texto é sincronizado com `value` por um `$effect` (reagente) em vez de um
  // inicializador local — assim muda quando o valor externo muda sem clobber a
  // digitação (só quando o campo não está em foco).
  let text = $state("");
  let invalid = $state(false);
  let el: HTMLInputElement | undefined = $state();

  $effect(() => {
    if (el && document.activeElement !== el) {
      text = fmt(value);
      invalid = false;
    }
  });

  function commit(): void {
    const parsed = parse(text);
    if (parsed === null) {
      invalid = true;
      return;
    }
    invalid = false;
    text = fmt(parsed);
    if (parsed !== value) onCommit(parsed);
  }

  function cancel(): void {
    text = fmt(value);
    invalid = false;
  }
</script>

<input
  bind:this={el}
  class:invalid
  class="timing"
  value={text}
  aria-label={ariaLabel}
  aria-invalid={invalid}
  inputmode="numeric"
  spellcheck="false"
  onchange={commit}
  onkeydown={(e) => {
    if (e.key === "Enter") commit();
    if (e.key === "Escape") cancel();
  }}
/>

<style>
  .timing {
    font: inherit;
    font-variant-numeric: tabular-nums;
    color: var(--text);
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    padding: 2px var(--space-1);
    width: 92px;
  }

  .timing:hover {
    border-color: var(--border);
  }

  .timing:focus {
    outline: 2px solid var(--focus-ring);
    border-color: var(--accent);
    background: var(--surface);
  }

  .timing.invalid {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
