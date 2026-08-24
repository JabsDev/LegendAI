## Descrição

<!-- O que este PR faz? Referencie a issue/tarefa do PLANNING.md quando aplicável. -->

## Mudanças de comportamento

<!-- Liste o que muda para o usuário/desenvolvedor. "Nenhuma" se for interno. -->

## Validação

<!-- Evidências de que as mudanças funcionam: testes rodados, resultados de clippy/fmt, screenshots de UI. -->

- [ ] `cargo fmt -- --check` passa
- [ ] `cargo clippy --all-targets -- -D warnings` passa
- [ ] `cargo test` (e `--features stt` quando aplicável) passa
- [ ] `npm run lint`, `npm run check` e `npm run build` passam (para mudanças de frontend)

## Checklist

- [ ] Mudança pequena e coesa (um PR = uma tarefa/mudança)
- [ ] Testes adicionados/atualizados para o que mudou
- [ ] Docs afetados atualizados (`docs/`, `README.md`, `CONTRIBUTING.md`)
- [ ] `PLANNING.md` atualizado (status da tarefa, notas) se aplicável
- [ ] Nenhum modelo/binário commitado indevidamente (`src-tauri/binaries/`, `.secrets/`)

## Screenshots (opcional)

<!-- Para mudanças de UI: antes/depois. -->
