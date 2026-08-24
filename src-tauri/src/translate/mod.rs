pub mod batcher;
pub mod engine;
pub mod factory;
pub mod glossary;
#[cfg(feature = "llama")]
pub mod llm;
#[cfg(feature = "ort")]
pub mod nllb;
pub mod options;
pub mod parser;
pub mod prompt;

#[allow(unused_imports)] // API pública consumida por 3.6/3.10
pub use batcher::{chunk_segments, Batch, DEFAULT_BATCH_SIZE, DEFAULT_CONTEXT_SIZE};
#[allow(unused_imports)] // API pública consumida por 3.2/3.3/3.4/3.10
pub use engine::{
    BatchOptions, BatchRequest, BatchResult, BatchSegment, MockEngine, TranslateError,
    TranslatedSegment, TranslationEngine, TranslationStatus,
};
#[allow(unused_imports)] // consumido por 3.8/3.10
pub use factory::TranslationEngineFactory;
#[allow(unused_imports)] // glossário persistente consumido por commands/config.rs (5.6)
pub use glossary::{Glossary, GlossaryEntry};
#[allow(unused_imports)] // opções persistidas na config e injetadas no prompt (5.4)
pub use options::{Formality, TranslationOptions};
#[allow(unused_imports)] // consumido por 3.10 (pipeline LLM)
pub use parser::{parse_batch_response, translate_with_retry};
#[allow(unused_imports)] // build_prompt consumido por 3.3/3.10 (GlossaryEntry do prompt
// via `prompt::GlossaryEntry`; o nome `GlossaryEntry` na raiz é o do glossário 5.6)
pub use prompt::{build_prompt, LanguagePair};

impl From<engine::TranslateError> for crate::errors::LegendaiError {
    fn from(e: engine::TranslateError) -> Self {
        let engine::TranslateError::Backend(msg) = e;
        if msg.contains("feature") {
            crate::errors::LegendaiError::TranslateFeatureMissing(msg)
        } else if msg.contains("indisponível")
            || msg.contains("não foi baixado")
            || msg.contains("não existe no catálogo")
        {
            crate::errors::LegendaiError::TranslateUnavailable(msg)
        } else {
            crate::errors::LegendaiError::TranslateFailed(msg)
        }
    }
}
