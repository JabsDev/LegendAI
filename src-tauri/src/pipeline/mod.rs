pub mod embedded;
pub mod memory;
pub mod queue;
pub mod steps;
pub mod stt_pipeline;
pub mod translate_pipeline;

#[allow(unused_imports)] // extração de legenda embutida consumida pela 3.10, 4.2 e 4.3
pub use embedded::{extract_subtitle, load_embedded_subtitle, EmbeddedError};
#[allow(unused_imports)] // API pública consumida pelo teste E2E (1.9) e comandos IPC
pub use stt_pipeline::{run_stt, SttPipelineOptions, SttResult};
#[allow(unused_imports)] // swap de memória (3.8), pipeline de tradução (3.10) e job da 4.3
pub use translate_pipeline::{
    run_transcribe_and_swap, run_translate, run_translate_with_engine,
    run_translate_with_engine_progress, TranslatePipelineError, TranslateResult,
    TranslateSwapResult,
};
