import pt from "./pt.json";
import en from "./en.json";
import { savePrefs } from "../lib/prefs.svelte";

export type Lang = "pt" | "en";
export type Messages = Record<string, string>;

const STORAGE_KEY = "legendai-lang";

// Runes exigem `.svelte.ts` (a extensão `index.ts` do plano não é processada
// pelo compilador do Svelte) — mesma API, arquivo de loader + store.
const messages: Record<Lang, Messages> = { pt, en };

function initialLang(): Lang {
  const stored = localStorage.getItem(STORAGE_KEY);
  return stored === "en" ? "en" : "pt";
}

const initial = initialLang();
let current = $state<Lang>(initial);

document.documentElement.lang = initial;

export function getLang(): Lang {
  return current;
}

export function setLang(lang: Lang): void {
  current = lang;
  localStorage.setItem(STORAGE_KEY, lang);
  document.documentElement.lang = lang;
  savePrefs({ ui_language: lang });
}

export { messages };
