fn main() {
    tauri_build::build();

    // Garante que o instalador não seja distribuído sem os sidecars
    // ffmpeg/ffprobe. Em `tauri dev` (PROFILE=debug) apenas avisa, para não
    // bloquear testes sem binário (que fazem skip). Em `tauri build`
    // (PROFILE=release) falha com instrução clara — o usuário final nunca
    // deve ver "sidecar não encontrado" em produção.
    let profile = std::env::var("PROFILE").unwrap_or_default();
    let is_release = profile == "release";

    // `tauri_build` já valida `externalBin`, mas a mensagem padrão é genérica.
    // Validamos aqui com o nome exato esperado (com .exe no Windows) para
    // dar a dica de `scripts/fetch-ffmpeg.sh`.
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let binaries_dir = manifest_dir.join("binaries");
    // TARGET é exposto pelo Cargo quando cross-compilando (ex: aarch64-pc-windows-msvc).
    // Se não houver, apenas verifica se existe *algum* sidecar para o nome.
    let target = std::env::var("TARGET").unwrap_or_default();
    let expected_ext = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let mut missing = Vec::new();
    for name in ["ffmpeg", "ffprobe"] {
        let has_any = binaries_dir
            .read_dir()
            .map(|mut d| {
                d.any(|e| {
                    e.map(|e| {
                        let n = e.file_name().to_string_lossy().to_string();
                        n.starts_with(&format!("{name}-"))
                    })
                    .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        // Se TARGET conhecido, verifica arquivo exato; senão, basta ter algum.
        let has_exact = if target.is_empty() {
            has_any
        } else {
            let expected = binaries_dir.join(format!("{name}-{target}{expected_ext}"));
            expected.exists()
        };
        if !has_exact {
            // Mensagem mostra o diretório, não um triple específico quando TARGET vazio
            let hint = if target.is_empty() {
                format!("{}/{}-*", binaries_dir.display(), name)
            } else {
                binaries_dir
                    .join(format!("{name}-{target}{expected_ext}"))
                    .display()
                    .to_string()
            };
            if !has_any {
                missing.push(hint);
            }
        }
    }

    if !missing.is_empty() {
        let msg = format!(
            "sidecar ffmpeg/ffprobe não encontrado em src-tauri/binaries/ (faltando: {}). \
             Em dev rode `bash scripts/fetch-ffmpeg.sh` (ou `bash scripts/fetch-ffmpeg.sh win64` no Windows/Git Bash). \
             Em CI o workflow já faz o fetch antes do `tauri build` — não distribua instalador sem os binários.",
            missing.join(", ")
        );
        if is_release {
            panic!("{msg}");
        } else {
            println!("cargo:warning={msg}");
        }
    }

    // Link CUDA para a feature `cuda` no Linux (variante nvidia do release).
    //
    // O build script do `llama-cpp-sys-4` em modo ESTÁTICO não emite
    // `cargo:rustc-link-lib` para cudart/cublas — o CMake liga
    // `CUDA::cudart_static` PRIVATE no ggml-cuda mas o target não é instalado
    // e o `extract_lib_names` só varre libllama/libggml*.a. O resultado era
    // "undefined symbol: cudaGetDeviceCount" no link do binário. Emitimos
    // aqui os links do runtime CUDA (não versionados — resolvem no toolkit
    // do CI e no driver do usuário via link dinâmico de libcuda.so.1).
    let target = std::env::var("TARGET").unwrap_or_default();
    let cuda_enabled = std::env::var("CARGO_FEATURE_CUDA").is_ok();
    if cuda_enabled && target.contains("linux") {
        if let Some(cuda_path) = std::env::var_os("CUDA_PATH") {
            let lib64 = std::path::Path::new(&cuda_path).join("lib64");
            if lib64.exists() {
                println!("cargo:rustc-link-search=native={}", lib64.display());
            }
        }
        println!("cargo:rustc-link-lib=cudart");
        println!("cargo:rustc-link-lib=cublas");
        println!("cargo:rustc-link-lib=cublasLt");
    }
}
