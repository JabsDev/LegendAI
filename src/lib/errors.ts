// Mapeamento código de erro → i18n + ação (tarefa 4.8).
//
// O backend envia `{ code, message, hint }` (via `LegendaiError::to_detail` da
// 1.10 no `pipeline-finished`, e como string em comandos simples). Este módulo:
//
//   - `toToast(e, logPath)` converte qualquer rejeição de IPC (objeto `{code,
//     message, hint}` OU string) num `ErrorToastInfo` pronto para exibir;
//   - `CODE_SPECS` mapeia cada código estável de `LegendaiError` para chave
//     i18n de título/mensagem + severidade + ação opcional (critério "cada
//     variante de `LegendaiError` tem mensagem i18n mapeada");
//   - códigos desconhecidos (`pipeline_failed` e afins) caem no dialog de
//     "erro inesperado" (log path + issue no GitHub, critério 4.8);
//   - `errMsg(e, fallbackKey)` devolve a mensagem exibível (usada pelas telas
//     que mantêm erro inline além do toast).

import { t } from "./t";

export interface ErrorDetail {
  code: string;
  message: string;
  hint: string | null;
}

export type ToastSeverity = "error" | "warning";

export interface ToastAction {
  label: string; // chave i18n do rótulo do botão
  route: string; // rota do App para navegar (ex: "models")
}

export interface ErrorToastInfo {
  id: number;
  title: string;
  message: string;
  hint: string | null;
  severity: ToastSeverity;
  action: ToastAction | null;
  /// true → exibe o dialog de erro inesperado (log path + issue), não um toast.
  unexpected: boolean;
  logPath: string;
}

interface CodeSpec {
  titleKey: string;
  messageKey: string;
  severity: ToastSeverity;
  action?: ToastAction;
}

// Mapa dos códigos estáveis de `LegendaiError` (errors.rs `to_detail`). Cada
// variante tem título e mensagem i18n (pt/en) — a UI nunca exibe o `message`
// pt-BR do backend quando o código é conhecido.
const CODE_SPECS: Record<string, CodeSpec> = {
  no_audio_track: {
    titleKey: "errors.noAudioTrack.title",
    messageKey: "errors.noAudioTrack.message",
    severity: "warning",
  },
  corrupted_file: {
    titleKey: "errors.corruptedFile.title",
    messageKey: "errors.corruptedFile.message",
    severity: "error",
  },
  ffmpeg_missing: {
    titleKey: "errors.ffmpegMissing.title",
    messageKey: "errors.ffmpegMissing.message",
    severity: "error",
  },
  model_missing: {
    titleKey: "errors.modelMissing.title",
    messageKey: "errors.modelMissing.message",
    severity: "error",
    action: { label: "errors.actionOpenModels", route: "models" },
  },
  model_corrupt: {
    titleKey: "errors.modelCorrupt.title",
    messageKey: "errors.modelCorrupt.message",
    severity: "error",
    action: { label: "errors.actionOpenModels", route: "models" },
  },
  stt_model_unavailable: {
    titleKey: "errors.sttModelUnavailable.title",
    messageKey: "errors.sttModelUnavailable.message",
    severity: "warning",
    action: { label: "errors.actionOpenModels", route: "models" },
  },
  no_speech: {
    titleKey: "errors.noSpeech.title",
    messageKey: "errors.noSpeech.message",
    severity: "warning",
  },
  unsupported_language: {
    titleKey: "errors.unsupportedLanguage.title",
    messageKey: "errors.unsupportedLanguage.message",
    severity: "error",
  },
  transcribe_failed: {
    titleKey: "errors.transcribeFailed.title",
    messageKey: "errors.transcribeFailed.message",
    severity: "error",
  },
  translate_unavailable: {
    titleKey: "errors.translateUnavailable.title",
    messageKey: "errors.translateUnavailable.message",
    severity: "warning",
    action: { label: "errors.actionOpenModels", route: "models" },
  },
  translate_feature_missing: {
    titleKey: "errors.translateFeatureMissing.title",
    messageKey: "errors.translateFeatureMissing.message",
    severity: "error",
  },
  translate_failed: {
    titleKey: "errors.translateFailed.title",
    messageKey: "errors.translateFailed.message",
    severity: "error",
  },
  config_dir_missing: {
    titleKey: "errors.configDirMissing.title",
    messageKey: "errors.configDirMissing.message",
    severity: "error",
  },
  io_error: {
    titleKey: "errors.ioError.title",
    messageKey: "errors.ioError.message",
    severity: "error",
  },
  config_invalid: {
    titleKey: "errors.configInvalid.title",
    messageKey: "errors.configInvalid.message",
    severity: "warning",
  },
  config_serialize: {
    titleKey: "errors.configSerialize.title",
    messageKey: "errors.configSerialize.message",
    severity: "error",
  },
};

let nextId = 1;

// Converte a rejeição de um comando IPC em `ErrorToastInfo`.
// `e` pode ser: objeto `{ code, message, hint }` (pipeline-finished) ou uma
// string (comandos `Result<_, String>` do Tauri). Strings sem código viram um
// toast simples (a mensagem já é acionável); objetos com código mapeado usam
// i18n; objetos com código desconhecido viram "erro inesperado" (dialog).
export function toToast(e: unknown, logPath: string): ErrorToastInfo {
  const detail = parseDetail(e);
  if (detail) {
    const spec = CODE_SPECS[detail.code];
    if (spec) {
      return {
        id: nextId++,
        title: t(spec.titleKey),
        // `unsupported_language` carrega o código de idioma dinâmico na
        // mensagem do backend — usá-la mantém a informação específica.
        message:
          detail.code === "unsupported_language" && detail.message
            ? detail.message
            : t(spec.messageKey),
        hint: detail.hint ?? null,
        severity: spec.severity,
        action: spec.action ?? null,
        unexpected: false,
        logPath,
      };
    }
    // Código desconhecido (ex: `pipeline_failed`) → erro inesperado.
    return {
      id: nextId++,
      title: t("errors.unexpected.title"),
      message: detail.message || t("errors.unexpected.message"),
      hint: detail.hint ?? null,
      severity: "error",
      action: null,
      unexpected: true,
      logPath,
    };
  }
  const raw = typeof e === "string" && e ? e : null;
  return {
    id: nextId++,
    title: t("errors.genericTitle"),
    message: raw ?? t("errors.unexpected.message"),
    hint: null,
    severity: "error",
    action: null,
    unexpected: false,
    logPath,
  };
}

// Mensagem exibível de um erro (usada por telas que mantêm erro inline).
export function errMsg(e: unknown, fallbackKey: string): string {
  const raw = typeof e === "string" && e ? e : null;
  if (raw) return raw;
  const info = toToast(e, "");
  return info.unexpected ? t(fallbackKey) : info.message;
}

function parseDetail(e: unknown): ErrorDetail | null {
  if (e && typeof e === "object" && "code" in e && "message" in e) {
    const d = e as ErrorDetail;
    return typeof d.code === "string" ? d : null;
  }
  if (typeof e === "string") {
    try {
      const parsed = JSON.parse(e);
      if (parsed && typeof parsed === "object" && typeof parsed.code === "string") {
        return parsed as ErrorDetail;
      }
    } catch {
      /* não é JSON — tratar como string simples */
    }
  }
  return null;
}

// URL base da issue pré-preenchida do GitHub (nota da 4.8).
// `ponytail:` dono do repo ainda não definido (sem remote configurado) —
// trocar `<user>` quando o repositório for publicado (tarefa 6.7).
export const GITHUB_ISSUES_BASE = "https://github.com/<user>/legendai/issues/new";
