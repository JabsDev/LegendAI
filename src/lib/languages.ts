export interface LangOption {
  code: string;
  label: string;
}

export const TARGET_LANGS: LangOption[] = [
  { code: "pt", label: "Português" },
  { code: "en", label: "English" },
  { code: "es", label: "Español" },
  { code: "fr", label: "Français" },
  { code: "de", label: "Deutsch" },
  { code: "it", label: "Italiano" },
  { code: "ja", label: "日本語" },
  { code: "zh", label: "中文" },
  { code: "ar", label: "العربية" },
  { code: "ru", label: "Русский" },
  { code: "ko", label: "한국어" },
  { code: "nl", label: "Nederlands" },
  { code: "pl", label: "Polski" },
  { code: "tr", label: "Türkçe" },
  { code: "hi", label: "हिन्दी" },
];

export function langLabel(code: string): string {
  return TARGET_LANGS.find((l) => l.code === code)?.label ?? code.toUpperCase();
}
