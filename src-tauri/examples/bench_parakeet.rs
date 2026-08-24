//! Bench manual Parakeet TDT: transcreve um WAV 16k mono e mostra segmentos + tempo.
//! Uso: cargo run --features full,cuda --example bench_parakeet -- /tmp/bench_30s.wav

use std::path::Path;
use std::time::Instant;

use legendai_lib::stt::parakeet::ParakeetModel;

fn main() {
    let wav = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/bench_30s.wav".into());
    let dir = "/home/jabs/.cache/legendai/models/stt/parakeet-tdt-0.6b-v3";
    println!("carregando Parakeet de {dir}...");
    let start = Instant::now();
    let mut model = ParakeetModel::load(Path::new(dir), 8, true).expect("load parakeet");
    println!("load: {}ms", start.elapsed().as_millis());
    let t0 = Instant::now();
    let t = model
        .transcribe(Path::new(&wav), None)
        .expect("transcrever");
    let secs = t0.elapsed().as_secs_f64();
    println!(
        "transcrito em {:.2}s ({} segs, lang {})",
        secs,
        t.segments.len(),
        t.language.as_code()
    );
    for s in t.segments.iter().take(15) {
        println!(
            "[{:.1}s-{:.1}s] {}",
            s.start_ms.as_secs_f64(),
            s.end_ms.as_secs_f64(),
            s.text
        );
    }
}
