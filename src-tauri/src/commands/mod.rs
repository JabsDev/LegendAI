//! Comandos IPC expostos à UI (tarefa 2.8 em diante).
//!
//! Cada submodulo agrupa comandos de uma área (modelos, pipeline, config).
//! Comandos aqui são consumidos via `@tauri-apps/api/core` `invoke` no
//! frontend (ver `src/components/models/ModelList.svelte`).

pub mod app;
pub mod config;
pub mod export;
pub mod import;
pub mod models;
pub mod onboarding;
#[cfg(feature = "stt")]
pub mod pipeline;
pub mod preview;
pub mod subtitles;
