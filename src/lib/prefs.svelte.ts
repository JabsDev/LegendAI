import { invoke } from "@tauri-apps/api/core";

// Store reativo das preferências do usuário (tarefa 4.10). O backend
// (`commands/config.rs`) é a fonte de verdade persistida; o localStorage é
// apenas o cache rápido aplicado antes do mount (evita flash de tema/idioma).
// `savePrefs` usa debounce de 500ms para não escrever disco a cada mudança.

export interface LanguagePair {
  source: string;
  target: string;
}

/** Opções avançadas de tradução (tarefa 5.4) — persistidas no backend. */
export interface TranslationOptions {
  formality: "formal" | "colloquial";
  custom_instructions: string;
  context_size: number;
}

export interface Prefs {
  theme: string;
  ui_language: string;
  preview_mode: string;
  last_output_dir: string | null;
  last_language_pair: LanguagePair | null;
  recent_files: string[];
  translation_options: TranslationOptions;
}

export interface PrefsPatch {
  theme?: string;
  ui_language?: string;
  preview_mode?: string;
  last_output_dir?: string;
  last_language_pair?: LanguagePair;
  recent_file?: string;
  translation_options?: TranslationOptions;
}

/** Entrada do glossário do usuário (tarefa 5.6), espelha `translate/glossary.rs`. */
export interface GlossaryEntry {
  term: string;
  translation: string;
  note?: string | null;
}

const THEME_KEY = "legendai-theme";

// Síncrono: o backend ainda não respondeu no boot — usa o cache local.
function initialTheme(): string {
  return localStorage.getItem(THEME_KEY) === "dark" ? "dark" : "light";
}

const DEFAULT_PREFS: Prefs = {
  theme: initialTheme(),
  ui_language: "pt",
  preview_mode: "translated",
  last_output_dir: null,
  last_language_pair: null,
  recent_files: [],
  translation_options: { formality: "colloquial", custom_instructions: "", context_size: 3 },
};

let prefs = $state<Prefs>(DEFAULT_PREFS);

export function getPrefs(): Prefs {
  return prefs;
}

export async function loadPrefs(): Promise<void> {
  try {
    prefs = await invoke<Prefs>("get_prefs");
  } catch (e) {
    console.error("falha ao carregar preferências", e);
  }
}

let timer: ReturnType<typeof setTimeout> | undefined;

export function savePrefs(patch: PrefsPatch): void {
  clearTimeout(timer);
  timer = setTimeout(() => {
    invoke<Prefs>("set_prefs", { patch })
      .then((p) => (prefs = p))
      .catch((e) => console.error("falha ao salvar preferências", e));
  }, 500);
}

/** Aplica o tema no DOM + localStorage e persiste no backend (debounced). */
export function setTheme(t: string): void {
  if (t === "dark") document.documentElement.dataset.theme = "dark";
  else delete document.documentElement.dataset.theme;
  localStorage.setItem(THEME_KEY, t);
  savePrefs({ theme: t });
}

// ── Glossário (tarefa 5.6) ──────────────────────────────────────────────────
// O glossário é persistido em `glossary.toml` (arquivo separado da config), por
// isso tem comandos próprios. `saveGlossary` usa debounce de 500ms: envia a
// lista inteira a cada mudança, então a edição rápida de várias linhas colapsa
// numa única escrita atômica.

export async function loadGlossary(): Promise<GlossaryEntry[]> {
  try {
    return await invoke<GlossaryEntry[]>("get_glossary");
  } catch (e) {
    console.error("falha ao carregar glossário", e);
    return [];
  }
}

let glossaryTimer: ReturnType<typeof setTimeout> | undefined;

export function saveGlossary(entries: GlossaryEntry[]): void {
  clearTimeout(glossaryTimer);
  glossaryTimer = setTimeout(() => {
    invoke<GlossaryEntry[]>("set_glossary", { entries }).catch((e) =>
      console.error("falha ao salvar glossário", e),
    );
  }, 500);
}
