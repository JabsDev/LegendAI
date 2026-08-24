// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Smoke test (6.8): roda checks (ffmpeg, config, onboarding) sem abrir a
    // GUI e sai com código — usado no CI pós-release e em diagnóstico.
    if std::env::args().any(|a| a == "--smoke-test") {
        std::process::exit(legendai_lib::smoke::run());
    }
    legendai_lib::logging::init();
    legendai_lib::run()
}
