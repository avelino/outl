---
description: Roda fmt + clippy + test + doc do workspace inteiro. Use antes de reportar done.
allowed-tools: Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*), Bash(cargo build:*), Bash(cargo doc:*), Bash(RUSTDOCFLAGS=*:*)
---

Rode em sequência e reporte resultado de cada etapa:

1. `cargo fmt --all -- --check` — formato
2. `cargo clippy --workspace --all-targets -- -D warnings` — lints
3. `cargo test --workspace --all-targets` — testes
4. `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — docs
   (CI roda isso; quebra em links de intra-doc pra items privados, p.ex.
   ``[`Foo`]`` onde `Foo` é `pub(crate)`. Drop os brackets: `` `Foo` ``.)

Se algum falhar, **pare** e mostre a saída exata. Não tente corrigir automaticamente — só relate.

Formato de saída:

```
fmt:     PASS | FAIL (N arquivos)
clippy:  PASS | FAIL (N warnings)
test:    PASS | FAIL (N falhas)
doc:     PASS | FAIL (N warnings)

[detalhes da falha, se houver]
```
