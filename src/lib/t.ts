import { getLang, messages } from "../i18n/index.svelte";

export type Vars = Record<string, string | number>;

// `t('key')` lê o sinal reativo do idioma (runes do `i18n/index.svelte.ts`),
// então chamá-lo no template re-renderiza ao trocar o idioma — sem refresh.
export function t(key: string, vars?: Vars): string {
  const msg = messages[getLang()][key] ?? key;
  if (!vars) return msg;
  return msg.replace(/\$\{(\w+)\}/g, (match, name: string) =>
    name in vars ? String(vars[name]) : match,
  );
}
