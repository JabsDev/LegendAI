use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::audio::ffmpeg_extract::extract_wav;
use crate::domain::{Segment, Subtitle, Timestamp};
use crate::errors::LegendaiError;
use crate::format::{format_subtitles, FormattedSubtitle};
use crate::stt::{SttOptions, Transcription, WhisperModel};
use crate::subtitles::srt::to_srt;

/// Opções do pipeline: trilha de áudio a extrair + opções do Whisper.
#[derive(Debug, Clone, Default)]
pub struct SttPipelineOptions {
    pub audio_track: usize,
    pub stt: SttOptions,
}

/// Resultado do pipeline: transcrição bruta, legenda, versão formatada, SRT
/// final e a duração do áudio fonte (para validar o timing).
#[derive(Debug)]
pub struct SttResult {
    pub transcription: Transcription,
    pub subtitle: Subtitle,
    pub formatted: Vec<FormattedSubtitle>,
    pub srt: String,
    pub audio_duration: Duration,
}

/// Orquestra o fluxo STT: extrai o WAV 16kHz mono (1.1), transcreve com o
/// Whisper (1.4), aplica as regras profissionais (1.8) e serializa SRT (1.7).
///
/// `input` pode ser um vídeo OU um WAV já extraído — o ffmpeg re-encoda para
/// 16kHz mono de forma idempotente (um WAV já nesse formato passa sem perda).
/// O WAV temporário fica em temp dir e é removido ao final (sucesso ou erro).
pub fn run_stt(
    model: &WhisperModel,
    input: &Path,
    opts: &SttPipelineOptions,
) -> Result<SttResult, LegendaiError> {
    let temp_dir = std::env::temp_dir().join(format!("legendai-pipeline-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).map_err(|source| LegendaiError::Io {
        path: temp_dir.to_string_lossy().into_owned(),
        source,
    })?;
    let _guard = TempCleanup(temp_dir.clone());
    let wav = temp_dir.join("audio.wav");

    let (_, audio_duration) = extract_wav(input, opts.audio_track, &wav)?;
    let transcription = model.transcribe(&wav, &opts.stt)?;
    if transcription.segments.is_empty() {
        return Err(LegendaiError::NoSpeech);
    }

    let subtitle = Subtitle {
        index: 1,
        segments: transcription.segments.clone(),
        language: transcription.language.clone(),
    };
    // Cada segmento do Whisper tem seu próprio timing — criar um Subtitle por
    // segmento preserva os gaps de silêncio. Um único Subtitle com todos os
    // segmentos seria tratado pelo `format_subtitles` como um span contínuo
    // (cursor proporcional), eliminando pausas e acelerando a legenda.
    let per_segment: Vec<Subtitle> = transcription
        .segments
        .iter()
        .enumerate()
        .map(|(i, seg)| Subtitle {
            index: (i + 1) as u32,
            segments: vec![seg.clone()],
            language: transcription.language.clone(),
        })
        .collect();
    let mut formatted = format_subtitles(&per_segment);
    clamp_to_audio(&mut formatted, audio_duration);
    let srt = to_srt(&formatted_to_subtitles(&formatted));

    Ok(SttResult {
        transcription,
        subtitle,
        formatted,
        srt,
        audio_duration,
    })
}

/// O whisper pode emitir fragmentos no tail além do fim do áudio (alucinação no
/// mel overhang), e o formatter estende o `end` da última legenda sem saber a
/// duração do áudio. Este pós-processo: descarta legendas que COMEÇAM além do
/// áudio, capa o `end` da última legenda na duração do áudio e re-indexa.
/// `pub(crate)`: reutilizado pelo job da tela de processamento (4.3) quando a
/// tradução está desligada.
pub(crate) fn clamp_to_audio(formatted: &mut Vec<FormattedSubtitle>, audio: Duration) {
    if formatted.is_empty() {
        return;
    }
    let audio_ms = audio.as_millis() as u64;
    formatted.retain(|f| f.start_ms.as_ms() < audio_ms);
    if let Some(last) = formatted.last_mut() {
        if last.end_ms.as_ms() > audio_ms {
            let end = audio_ms.max(last.start_ms.as_ms().saturating_add(1));
            last.end_ms = Timestamp::from_ms(end);
        }
    }
    for (i, f) in formatted.iter_mut().enumerate() {
        f.index = i as u32 + 1;
    }
}

/// Converte legendas formatadas em `Subtitle` (um segmento por linha) para
/// serialização SRT — cada linha vira uma linha de texto do bloco.
/// `pub(crate)`: reutilizado pelo pipeline de tradução (3.10) no mesmo passo 1.7.
pub(crate) fn formatted_to_subtitles(formatted: &[FormattedSubtitle]) -> Vec<Subtitle> {
    formatted
        .iter()
        .map(|f| Subtitle {
            index: f.index,
            segments: f
                .lines
                .iter()
                .map(|l| Segment {
                    text: l.clone(),
                    start_ms: f.start_ms,
                    end_ms: f.end_ms,
                    lang: f.language.clone(),
                })
                .collect(),
            language: f.language.clone(),
        })
        .collect()
}

/// Remove o diretório temporário do pipeline ao sair do escopo.
struct TempCleanup(PathBuf);

impl Drop for TempCleanup {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs(idx: u32, start: u64, end: u64) -> FormattedSubtitle {
        FormattedSubtitle {
            index: idx,
            lines: vec!["texto".into()],
            start_ms: Timestamp::from_ms(start),
            end_ms: Timestamp::from_ms(end),
            language: crate::domain::Language::Pt,
        }
    }

    #[test]
    fn clamp_nao_altere_legendas_dentro_da_duracao() {
        let mut f = vec![fs(1, 1000, 2000)];
        clamp_to_audio(&mut f, Duration::from_secs(10));
        assert_eq!(f[0].end_ms.as_ms(), 2000);
        assert_eq!(f[0].index, 1);
    }

    #[test]
    fn clamp_limita_ultimo_end_a_duracao_do_audio() {
        let mut f = vec![fs(1, 1000, 5000)];
        clamp_to_audio(&mut f, Duration::from_millis(3000));
        assert_eq!(f[0].end_ms.as_ms(), 3000);
    }

    #[test]
    fn clamp_descarta_legenda_que_comeca_alem_do_audio_e_reindexa() {
        let mut f = vec![fs(1, 1000, 2000), fs(2, 3500, 3600)];
        clamp_to_audio(&mut f, Duration::from_millis(3000));
        assert_eq!(f.len(), 1, "legenda que começa além do áudio é descartada");
        assert_eq!(f[0].index, 1);
        assert_eq!(f[0].end_ms.as_ms(), 2000);
    }

    #[test]
    fn clamp_descarta_e_capa_o_novo_ultimo() {
        let mut f = vec![fs(1, 1000, 5000), fs(2, 3500, 3600)];
        clamp_to_audio(&mut f, Duration::from_millis(3000));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].index, 1);
        assert_eq!(f[0].end_ms.as_ms(), 3000);
    }

    #[test]
    fn clamp_vazio_nao_panica() {
        let mut f = Vec::new();
        clamp_to_audio(&mut f, Duration::from_secs(1));
        assert!(f.is_empty());
    }

    #[test]
    fn formatted_vira_subtitle_um_segmento_por_linha() {
        let formatted = vec![FormattedSubtitle {
            index: 3,
            lines: vec!["linha um".into(), "linha dois".into()],
            start_ms: Timestamp::from_ms(1000),
            end_ms: Timestamp::from_ms(3000),
            language: crate::domain::Language::Pt,
        }];
        let subs = formatted_to_subtitles(&formatted);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].index, 3);
        assert_eq!(subs[0].segments.len(), 2);
        assert_eq!(subs[0].segments[0].text, "linha um");
        assert_eq!(subs[0].segments[1].start_ms.as_ms(), 1000);
        assert_eq!(subs[0].segments[1].end_ms.as_ms(), 3000);
    }
}
