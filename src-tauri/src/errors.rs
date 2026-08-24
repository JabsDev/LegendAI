use thiserror::Error;

/// Detalhe de erro serializado para a UI (consumido pela tarefa 4.8).
/// `code` é estável entre versões — o frontend mapeia para i18n + ação
/// (ver ADR-006). `message` é o texto amigável de fallback (pt-BR), sem
/// caminhos internos; o contexto completo fica no log.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ErrorDetail {
    pub code: &'static str,
    pub message: String,
    pub hint: Option<&'static str>,
}

/// Erro central do LegendAI. Variantes de domínio carregam contexto
/// (`#[source]`); a UI só vê [`ErrorDetail`] via [`LegendaiError::to_detail`].
///
/// Regra da tarefa 1.10: mensagens estáveis e amigáveis, nunca expor caminhos
/// absolutos internos ao usuário — detalhes completos vão para o log (registrados
/// pelos `From` impls em `audio`/`stt`).
#[derive(Debug, Error)]
pub enum LegendaiError {
    #[error("diretório de configuração não encontrado: {0}")]
    ConfigDirMissing(String),
    #[error("erro de I/O em `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("config TOML inválida em `{path}`: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("falha ao serializar config: {0}")]
    Serialize(#[from] toml::ser::Error),

    // ── Fluxo STT (tarefa 1.10) ────────────────────────────────────────────
    /// O vídeo não tem trilha de áudio, ou a trilha pedida não existe.
    #[error("o vídeo não contém trilha de áudio")]
    NoAudioTrack,
    /// Arquivo de mídia inválido/corrompido, ou WAV extraído ilegível.
    #[error("o arquivo de mídia está corrompido ou em formato não suportado")]
    CorruptedFile,
    /// Sidecar ffmpeg/ffprobe ausente ou não executável.
    #[error("ffmpeg/ffprobe não está disponível neste sistema")]
    FfmpegMissing,
    /// Modelo de transcrição não encontrado no caminho configurado.
    #[allow(dead_code)] // alcançadas via From<SttError> (feature `stt`) e pipeline 1.9
    #[error("o modelo de transcrição não foi encontrado")]
    ModelMissing,
    /// Arquivo do modelo existe mas não carrega (corrompido/formato errado).
    #[allow(dead_code)] // alcançadas via From<SttError> (feature `stt`) e pipeline 1.9
    #[error("o modelo de transcrição está corrompido ou inválido")]
    ModelCorrupt,
    /// Transcrição não produziu nenhum segmento (áudio sem fala).
    #[allow(dead_code)] // alcançadas via From<SttError> (feature `stt`) e pipeline 1.9
    #[error("nenhuma fala detectada no áudio")]
    NoSpeech,
    /// Override de idioma inválido (código ISO 639-1 esperado).
    #[allow(dead_code)] // alcançadas via From<SttError> (feature `stt`) e pipeline 1.9
    #[error("idioma `{0}` não é suportado pela transcrição")]
    UnsupportedLanguage(String),
    /// Falha interna do runtime do Whisper (init/estado/decode).
    #[allow(dead_code)] // alcançadas via From<SttError> (feature `stt`) e pipeline 1.9
    #[error("falha ao executar a transcrição")]
    TranscribeFailed,

    // ── Fluxo de tradução (tarefa 3.10) ───────────────────────────────
    #[error("modelo de tradução indisponível: {0}")]
    TranslateUnavailable(String),
    #[error("tradução não compilada no binário: {0}")]
    TranslateFeatureMissing(String),
    #[error("falha ao traduzir: {0}")]
    TranslateFailed(String),
}

impl LegendaiError {
    /// Mapeia para [`ErrorDetail`] com código estável + mensagem amigável.
    /// Nenhuma variante expõe caminhos internos; detalhes ficam no log.
    #[allow(dead_code)] // consumida pelos comandos IPC (4.3) e mapeamento de erro da UI (4.8)
    pub fn to_detail(&self) -> ErrorDetail {
        let detail =
            |code: &'static str, message: &'static str, hint: Option<&'static str>| ErrorDetail {
                code,
                message: message.into(),
                hint,
            };
        match self {
            Self::ConfigDirMissing(_) => detail(
                "config_dir_missing",
                "O diretório de configuração não foi encontrado.",
                Some("Verifique as permissões de escrita do usuário."),
            ),
            Self::Io { .. } => detail("io_error", "Falha de leitura ou gravação em disco.", None),
            Self::Parse { .. } => detail(
                "config_invalid",
                "O arquivo de configuração está corrompido ou inválido.",
                Some("O aplicativo usará as configurações padrão."),
            ),
            Self::Serialize(_) => detail(
                "config_serialize",
                "Falha ao salvar as configurações.",
                None,
            ),
            Self::NoAudioTrack => detail(
                "no_audio_track",
                "O vídeo não contém trilha de áudio.",
                Some("Escolha outro arquivo ou uma trilha que contenha fala."),
            ),
            Self::CorruptedFile => detail(
                "corrupted_file",
                "O arquivo de mídia está corrompido ou em formato não suportado.",
                None,
            ),
            Self::FfmpegMissing => detail(
                "ffmpeg_missing",
                "O componente de mídia (ffmpeg) não está disponível neste sistema.",
                Some("Reinstale o aplicativo para restaurar os componentes de mídia."),
            ),
            Self::ModelMissing => detail(
                "model_missing",
                "O modelo de transcrição não foi encontrado.",
                Some("Baixe um modelo de transcrição na aba Modelos antes de transcrever."),
            ),
            Self::ModelCorrupt => detail(
                "model_corrupt",
                "O modelo de transcrição está corrompido ou inválido.",
                Some("Baixe o modelo novamente na aba Modelos."),
            ),
            Self::NoSpeech => detail(
                "no_speech",
                "Nenhuma fala foi detectada no áudio.",
                Some("Verifique se o vídeo contém fala ou escolha outra trilha de áudio."),
            ),
            Self::UnsupportedLanguage(code) => ErrorDetail {
                code: "unsupported_language",
                message: format!("O idioma `{code}` não é suportado pela transcrição."),
                hint: Some("Use um código ISO 639-1 (ex: pt, en, es)."),
            },
            Self::TranscribeFailed => detail(
                "transcribe_failed",
                "Falha ao executar a transcrição de áudio.",
                Some("Verifique o log do aplicativo para mais detalhes."),
            ),
            Self::TranslateUnavailable(_) => detail(
                "translate_unavailable",
                "O modelo de tradução não está disponível.",
                Some("Baixe e selecione um modelo de tradução na aba Modelos."),
            ),
            Self::TranslateFeatureMissing(_) => detail(
                "translate_feature_missing",
                "Tradução não disponível neste binário.",
                Some("Reinstale com suporte a tradução (build com --features full) ou escolha um modelo NLLB/Tower compatível."),
            ),
            Self::TranslateFailed(_) => detail(
                "translate_failed",
                "Falha ao executar a tradução.",
                Some("Verifique o log do aplicativo para mais detalhes."),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variantes_do_fluxo_stt_tem_codigo_estavel() {
        let cases = [
            (LegendaiError::NoAudioTrack, "no_audio_track"),
            (LegendaiError::CorruptedFile, "corrupted_file"),
            (LegendaiError::FfmpegMissing, "ffmpeg_missing"),
            (LegendaiError::ModelMissing, "model_missing"),
            (LegendaiError::ModelCorrupt, "model_corrupt"),
            (LegendaiError::NoSpeech, "no_speech"),
            (
                LegendaiError::UnsupportedLanguage("zz".into()),
                "unsupported_language",
            ),
            (LegendaiError::TranscribeFailed, "transcribe_failed"),
            (
                LegendaiError::TranslateUnavailable("x".into()),
                "translate_unavailable",
            ),
            (
                LegendaiError::TranslateFeatureMissing("x".into()),
                "translate_feature_missing",
            ),
            (
                LegendaiError::TranslateFailed("x".into()),
                "translate_failed",
            ),
        ];
        for (err, code) in cases {
            assert_eq!(
                err.to_detail().code,
                code,
                "código de `{err}` deve ser estável"
            );
        }
    }

    #[test]
    fn variantes_de_config_tem_codigo_estavel() {
        let io = std::io::Error::other("x");
        let parse = toml::from_str::<toml::Value>("]").unwrap_err();
        let cases = [
            (
                LegendaiError::ConfigDirMissing("razão".into()),
                "config_dir_missing",
            ),
            (
                LegendaiError::Io {
                    path: "/tmp/x".into(),
                    source: io,
                },
                "io_error",
            ),
            (
                LegendaiError::Parse {
                    path: "/tmp/x.toml".into(),
                    source: parse,
                },
                "config_invalid",
            ),
        ];
        for (err, code) in cases {
            assert_eq!(
                err.to_detail().code,
                code,
                "código de `{err}` deve ser estável"
            );
        }
    }

    #[test]
    fn mensagens_nao_expoem_caminhos_internos() {
        for err in [
            LegendaiError::NoAudioTrack,
            LegendaiError::CorruptedFile,
            LegendaiError::FfmpegMissing,
            LegendaiError::ModelMissing,
            LegendaiError::ModelCorrupt,
            LegendaiError::NoSpeech,
            LegendaiError::TranscribeFailed,
            LegendaiError::UnsupportedLanguage("zz".into()),
            LegendaiError::ConfigDirMissing("dirs::config_dir() retornou None".into()),
        ] {
            let d = err.to_detail();
            assert!(
                !d.message.contains('/') && !d.message.contains('\\'),
                "mensagem de `{err}` expõe caminho interno: {}",
                d.message
            );
        }
    }
}
