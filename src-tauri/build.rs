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

    // Link CUDA para a feature `cuda`. O build script do `llama-cpp-sys-4` em
    // modo ESTÁTICO não emite `cargo:rustc-link-lib` para cudart/cublas — o
    // CMake liga `CUDA::cudart*` PRIVATE no ggml-cuda mas o target não é
    // instalado e o `extract_lib_names` só varre libllama/libggml*.a(.lib).
    // O resultado era "unresolved external symbol cudaGetLastError" (Windows,
    // LNK2019) / "undefined symbol: cudaGetDeviceCount" (Linux, rust-lld) no
    // link final. Emitimos aqui os links do runtime CUDA:
    //   - Windows: `CUDA_PATH/lib/x64/cudart.lib` (import libs do toolkit do CI)
    //   - Linux: `CUDA_PATH/lib64` (não versionados, resolvem no link)
    let target = std::env::var("TARGET").unwrap_or_default();
    let cuda_enabled = std::env::var("CARGO_FEATURE_CUDA").is_ok();
    if cuda_enabled {
        if target.contains("windows") {
            if let Some(cuda_path) = std::env::var_os("CUDA_PATH") {
                let lib = std::path::Path::new(&cuda_path).join("lib").join("x64");
                if lib.exists() {
                    println!("cargo:rustc-link-search=native={}", lib.display());
                }
            }
            println!("cargo:rustc-link-lib=cudart");
            println!("cargo:rustc-link-lib=cublas");
            println!("cargo:rustc-link-lib=cublasLt");
        } else if target.contains("linux") {
            if let Some(cuda_path) = std::env::var_os("CUDA_PATH") {
                let lib64 = std::path::Path::new(&cuda_path).join("lib64");
                if lib64.exists() {
                    println!("cargo:rustc-link-search=native={}", lib64.display());
                }
            }
            // Também stubs do driver (CUDA_PATH/lib64/stubs) para o link — o
            // ggml-cuda usa a API do driver (cuDeviceGet, cuMemCreate...) p/
            // VMM; em runtime resolve contra a libcuda.so.1 do driver NVIDIA
            // instalado na máquina do usuário.
            if let Some(cuda_path) = std::env::var_os("CUDA_PATH") {
                let stubs = std::path::Path::new(&cuda_path).join("lib64").join("stubs");
                if stubs.exists() {
                    println!("cargo:rustc-link-search=native={}", stubs.display());
                }
            }
            println!("cargo:rustc-link-lib=cudart");
            println!("cargo:rustc-link-lib=cublas");
            println!("cargo:rustc-link-lib=cublasLt");
            println!("cargo:rustc-link-lib=cuda");
        }
    }
}
