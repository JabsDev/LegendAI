#![cfg(feature = "stt")]
//! Teste E2E do fluxo de transcrição (tarefa 1.9).
//!
//! Combina o pipeline completo (extração → transcrição → formatação → SRT)
//! sobre a fixture `tests/fixtures/audio-pt.wav` (fala pt-BR, ~23s) e valida o
//! SRT resultante contra as regras profissionais.
//!
//! Marcação `#[ignore]`: exige um modelo Whisper GGUF (env `LEGENDAI_MODEL_PATH`).
//! CI pula o teste se o modelo estiver ausente. Roda com:
//! `cargo test --features stt --test e2e_stt -- --ignored --nocapture`

use std::path::Path;

use legendai_lib::format::cps;
use legendai_lib::format::rules::{MAX_CHARS_PER_LINE, MAX_LINES, TARGET_CPS_MAX};
use legendai_lib::pipeline::{run_stt, SttPipelineOptions};
use legendai_lib::stt::WhisperModel;
use legendai_lib::subtitles::srt::parse_srt;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/audio-pt.wav");

#[test]
#[ignore = "exige modelo whisper GGUF via env LEGENDAI_MODEL_PATH (não roda em CI sem modelo)"]
fn e2e_transcricao_gera_srt_valido() {
    let model_path = std::env::var("LEGENDAI_MODEL_PATH")
        .expect("sete LEGENDAI_MODEL_PATH (GGUF do whisper) para rodar o teste E2E");
    let fixture = std::env::var("LEGENDAI_FIXTURE").unwrap_or_else(|_| FIXTURE.to_string());

    let model = WhisperModel::load(Path::new(&model_path))
        .unwrap_or_else(|e| panic!("falha ao carregar modelo: {e}"));
    let result = run_stt(&model, Path::new(&fixture), &SttPipelineOptions::default())
        .unwrap_or_else(|e| panic!("pipeline STT falhou: {e}"));

    // 1. Fala foi reconhecida (fixture não é silêncio).
    assert!(
        !result.formatted.is_empty(),
        "nenhuma legenda formatada — o áudio contém fala?"
    );
    let full_text: String = result
        .formatted
        .iter()
        .map(|f| f.text())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        full_text.split_whitespace().count() >= 10,
        "transcrição suspeitamente curta: {full_text:?}"
    );

    // 2. SRT final parseia e respeita as regras profissionais.
    let parsed = parse_srt(&result.srt).unwrap_or_else(|e| panic!("SRT final inválido: {e}"));
    assert!(!parsed.is_empty(), "SRT vazio");

    for sub in &parsed {
        assert!(
            sub.segments.len() <= MAX_LINES,
            "bloco {} com {} linhas (max {MAX_LINES})",
            sub.index,
            sub.segments.len()
        );
        for seg in &sub.segments {
            assert!(
                seg.text.chars().count() <= MAX_CHARS_PER_LINE,
                "linha com {} chars (max {MAX_CHARS_PER_LINE})",
                seg.text.chars().count()
            );
        }
        let start = sub.segments[0].start_ms.as_ms();
        let end = sub.segments[0].end_ms.as_ms();
        assert!(
            end > start,
            "bloco {} com end ({end}ms) <= start ({start}ms)",
            sub.index
        );
        let text: String = sub
            .segments
            .iter()
            .map(|s| s.text.clone())
            .collect::<Vec<_>>()
            .join(" ");
        let speed = cps(&text, end - start);
        assert!(
            speed <= TARGET_CPS_MAX + 0.001,
            "legenda {} com {speed:.1} cps acima do teto de {TARGET_CPS_MAX}",
            sub.index
        );
    }

    // 3. Sem sobreposição entre blocos consecutivos.
    for w in parsed.windows(2) {
        let a_end = w[0].segments.last().unwrap().end_ms.as_ms();
        let b_start = w[1].segments[0].start_ms.as_ms();
        assert!(
            a_end <= b_start,
            "overlap entre legendas {} (end {a_end}ms) e {} (start {b_start}ms)",
            w[0].index,
            w[1].index
        );
    }

    // 4. Timestamps coerentes com a duração do áudio: o pipeline capa o último
    //    `end` do SRT na duração (segmentos brutos do whisper podem exceder
    //    levemente — overhang do mel — e são corrigidos no pipeline).
    let last_end = parsed
        .last()
        .unwrap()
        .segments
        .last()
        .unwrap()
        .end_ms
        .as_ms();
    let audio_ms = result.audio_duration.as_millis() as u64;
    assert!(
        last_end <= audio_ms,
        "último timestamp {last_end}ms ultrapassa a duração do áudio {audio_ms}ms"
    );
    for pair in result.transcription.segments.windows(2) {
        assert!(
            pair[1].start_ms >= pair[0].end_ms,
            "segmentos brutos não monotônicos: {:?}..{:?} depois {:?}..{:?}",
            pair[0].start_ms,
            pair[0].end_ms,
            pair[1].start_ms,
            pair[1].end_ms
        );
    }

    eprintln!(
        "OK: idioma={} | legendas formatadas={} | duração={:.1}s | texto: {full_text:?}",
        result.transcription.language.as_code(),
        result.formatted.len(),
        result.audio_duration.as_secs_f64()
    );
}
