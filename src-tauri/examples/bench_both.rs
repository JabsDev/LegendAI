//! Bench comparativo: STT whisper-medium vs tiny, NLLB q4 vs LLM (qwen/tower) batched
//! Uso: cargo run --features full,cuda --example bench_both 2>&1 | tee /tmp/bench.log

use std::path::Path;
use std::time::Instant;

use legendai_lib::config::AppConfig;
use legendai_lib::hardware::detect::detect;
use legendai_lib::translate::TranslationEngineFactory;

fn main() {
    println!("=== LegendAI Bench Both ===");
    let hw = detect();
    println!(
        "HW: ram={}GB cpu={} threads={} gpu={:?}",
        hw.ram_gb, hw.cpu_threads, hw.recommended_threads, hw.gpu
    );
    println!("Tier: {:?}", legendai_lib::hardware::tier::tier_for(&hw));

    // 1. STT bench
    bench_stt();

    // 2. Translation bench (60 segs -> 6 lotes x10)
    bench_translation();
}

fn bench_stt() {
    println!("\n--- STT Bench (30s clip) ---");
    let video = "/mnt/sdb1/codes-ai/LegendAI/SteamOS Is Free Now… I Tried Daily Driving It [7EIk0crvc0k].mp4";
    if !Path::new(video).exists() {
        println!("video não encontrado: {}", video);
        return;
    }
    // extrai 10s de áudio para /tmp/bench.wav (bench rápido)
    let wav = "/tmp/bench_30s.wav";
    let out = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-ss",
            "60",
            "-i",
            video,
            "-t",
            "10",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
            wav,
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => println!("wav 30s extraído: {}", wav),
        Ok(o) => {
            println!("ffmpeg falhou: {}", String::from_utf8_lossy(&o.stderr));
            return;
        }
        Err(e) => {
            println!("ffmpeg erro: {}", e);
            return;
        }
    }
    let wav_path = Path::new(wav);
    // testa tiny (sempre presente) e medium (agora baixado); large não está mais no cache
    let candidates = [
        (
            "whisper-tiny",
            dirs::cache_dir()
                .unwrap()
                .join("legendai/models/whisper/ggml-tiny.bin"),
        ),
        (
            "whisper-medium-q5",
            dirs::cache_dir()
                .unwrap()
                .join("legendai/models/stt/whisper-medium-q5/ggml-medium-q5_0.bin"),
        ),
    ];
    for (id, p) in candidates {
        if !p.exists() {
            println!("\nSTT {} não baixado: {}", id, p.display());
            continue;
        }
        println!("\nSTT {} -> {}", id, p.display());
        let start = Instant::now();
        let whisper = match legendai_lib::stt::WhisperModel::load(&p) {
            Ok(m) => m,
            Err(e) => {
                println!("  load falhou: {}", e);
                continue;
            }
        };
        let load_ms = start.elapsed().as_millis();
        println!("  load: {}ms", load_ms);
        let opts = legendai_lib::stt::SttOptions {
            threads: 8,
            ..Default::default()
        };
        let t0 = Instant::now();
        match whisper.transcribe(wav_path, &opts) {
            Ok(t) => {
                let ms = t0.elapsed().as_millis();
                let secs = ms as f64 / 1000.0;
                let realtime = 10.0 / secs;
                println!(
                    "  transcribe 10s em {:.1}s -> {:.1}x realtime, {} segs, lang {}",
                    secs,
                    realtime,
                    t.segments.len(),
                    t.language.as_code()
                );
            }
            Err(e) => println!("  transcribe falhou: {}", e),
        }
    }
}

fn bench_translation() {
    println!("\n--- Translation Bench (20 segs, 2 lotes x10) ---");
    // segmentos sintéticos baseados no SRT real (primeiras linhas)
    let segs: Vec<String> = (0..20)
        .map(|i| {
            format!(
                "Este é o segmento de teste número {} com texto realista para legenda.",
                i + 1
            )
        })
        .collect();
    // cria BatchSegments
    let segments: Vec<legendai_lib::domain::Segment> = segs
        .iter()
        .enumerate()
        .map(|(i, t)| {
            legendai_lib::domain::Segment::new(
                t.clone(),
                legendai_lib::domain::Timestamp::from_ms(i as u64 * 2000),
                legendai_lib::domain::Timestamp::from_ms(i as u64 * 2000 + 1500),
                legendai_lib::domain::Language::Pt,
            )
            .unwrap()
        })
        .collect();
    let subtitles: Vec<legendai_lib::domain::Subtitle> = segments
        .chunks(1)
        .enumerate()
        .map(|(i, chunk)| legendai_lib::domain::Subtitle {
            index: i as u32 + 1,
            segments: chunk.to_vec(),
            language: legendai_lib::domain::Language::Pt,
        })
        .collect();

    let hw = detect();
    let mut cfg = AppConfig::load_or_default();
    let tests = [
        ("nllb-200-distilled-600m-q4", "NLLB q4 (GPU ONNX)"),
        ("nllb-200-distilled-600m", "NLLB fp16"),
        ("towerinstruct-7b-q4_k_m", "Tower 7B q4"),
        ("towerinstruct-7b-q6_k", "Tower 7B q6 (LLM batched)"),
    ];
    for (id, label) in tests {
        println!("\n{} ({}):", label, id);
        cfg.active_models.translation = id.to_string();
        cfg.target_lang = "en".to_string();
        let start = Instant::now();
        let mut engine = match TranslationEngineFactory::for_config(&cfg, &hw) {
            Ok(e) => e,
            Err(e) => {
                println!("  engine falhou (não baixado?): {}", e);
                continue;
            }
        };
        let load_ms = start.elapsed().as_millis();
        println!("  load: {}ms", load_ms);
        let t0 = Instant::now();
        match legendai_lib::pipeline::translate_pipeline::run_translate_with_engine(
            &mut *engine,
            &subtitles,
            &cfg,
        ) {
            Ok(res) => {
                let ms = t0.elapsed().as_millis();
                let secs = ms as f64 / 1000.0;
                let seg_s = 20.0 / secs;
                println!(
                    "  20 segs em {:.1}s -> {:.2} seg/s, kept {}",
                    secs, seg_s, res.kept_original_count
                );
                for (i, s) in res.subtitles.iter().take(2).enumerate() {
                    println!(
                        "    [{}] {}",
                        i + 1,
                        s.segments[0].text.chars().take(80).collect::<String>()
                    );
                }
            }
            Err(e) => println!("  translate falhou: {}", e),
        }
    }
    // qwen direto (não no catálogo) - testa LLM batched via llm::LlmEngine
    let qwen = Path::new("/home/jabs/.cache/legendai/models/translation/qwen2.5-7b-instruct-q4_k_m/qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf");
    if qwen.exists() {
        println!("\nQwen 7B q4 direto (bench LLM batched): {:?}", qwen);
        #[cfg(feature = "llama")]
        {
            use legendai_lib::translate::llm::LlmEngine;
            let threads = hw.recommended_threads as usize;
            let gpu_layers: u32 = 256;
            let start = Instant::now();
            match LlmEngine::load(qwen, threads, gpu_layers) {
                Ok(mut engine) => {
                    println!(
                        "  load: {}ms (threads {} gpu {})",
                        start.elapsed().as_millis(),
                        threads,
                        gpu_layers
                    );
                    let t0 = Instant::now();
                    let mut cfg2 = AppConfig::load_or_default();
                    cfg2.target_lang = "en".into();
                    cfg2.source_lang = "pt".into();
                    match legendai_lib::pipeline::translate_pipeline::run_translate_with_engine(
                        &mut engine,
                        &subtitles,
                        &cfg2,
                    ) {
                        Ok(res) => {
                            let secs = t0.elapsed().as_secs_f64();
                            println!(
                                "  Qwen 20 segs em {:.1}s -> {:.2} seg/s, kept {}",
                                secs,
                                20.0 / secs,
                                res.kept_original_count
                            );
                        }
                        Err(e) => println!("  Qwen translate falhou: {}", e),
                    }
                }
                Err(e) => println!("  Qwen load falhou: {}", e),
            }
        }
        #[cfg(not(feature = "llama"))]
        println!("  (feature llama não ativa)");
    }
}
