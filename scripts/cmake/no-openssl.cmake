# Toolchain CMake para builds macOS Intel (x86_64) a partir de runner arm64.
#
# O llama.cpp habilita HTTPS no cpp-httplib via OpenSSL por default
# (option(LLAMA_OPENSSL ... ON)). No runner Apple Silicon o find_package(OpenSSL)
# resolve para o Homebrew ARM64 (/opt/homebrew/Cellar/openssl@3), e o link
# cross-x86_64 de libllama-common.dylib falha com _ASN1_* / _ERR_* indefinidos.
#
# Este arquivo força LLAMA_OPENSSL=OFF: HTTPS do server do llama.cpp não é
# usado pelo LegendAI (o app só usa a API local de inferência), e a variável
# definida via toolchain é aplicada ANTES das option() do CMakeLists.
set(LLAMA_OPENSSL OFF CACHE BOOL "" FORCE)
