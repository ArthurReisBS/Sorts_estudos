# Visualizador de sorts


```bash
cargo run --release              # abre a janela
cargo run --release -- --verificar   # roda cada sort uma vez no terminal, sem janela
```

Precisa de `clang` e `rustc` no PATH. Só funciona em Linux x86_64 (ver "Como funciona").

## As duas abas

**Animação.** Escolha o sort, o N, o tipo de entrada e aperte **▶ Executar**. Os dois lados (C à esquerda, Rust à direita) recebem exatamente o mesmo vetor. Cada passo é um acesso ao vetor: amarelo é leitura e vermelho é escrita. Quando o sort termina, as barras ficam verdes se o vetor ficou ordenado. Embaixo aparece a tabela C × Rust com leituras, escritas e total de acessos.

**Comparação de tempo.** Mede o tempo real com o código otimizado (C com `clang -O3`, Rust com `opt-level=3`) para vários N. O C é compilado com clang, e não com gcc, porque o clang usa o mesmo LLVM do `rustc`: assim a diferença medida vem do código das duas linguagens, e não de dois compiladores diferentes. Cada ponto é a mediana de várias repetições, depois de uma rodada de aquecimento. Dá para medir só o sort escolhido ou todos de uma vez.

## O que o visualizador espera dos arquivos

Um arquivo por sort, com o mesmo nome nas duas pastas (`C/BubbleSort.c` e `Rust/BubbleSort.rs`). Arquivo vazio aparece como "ainda não implementado". Assinaturas aceitas:

| C | Rust |
| - | ---- |
| `void f(int *V, int N)` | `fn f(v: &mut [i32])` ou `fn f(v: &mut Vec<i32>)` |
| `void f(int *V, int inicio, int fim)` | `fn f(v: &mut [i32], n: usize)` |
| | `fn f(v: &mut [i32], inicio: usize, fim: usize)` |
| | `fn f(v: &[i32]) -> Vec<i32>` (devolve um vetor novo) |

- O vetor pode ser `int`, `long`, `unsigned` etc. em C, ou qualquer inteiro em Rust (`i32`, `i64`, `usize`...). Genérico (`fn f<T: Ord>(v: &mut [T])`) também funciona.
- A função chamada é a que tem o nome do arquivo (`QuickSort` → `quickSort` ou `quick_sort`). Se nenhuma tiver, ela é a única que nenhuma outra função do arquivo chama. Funções auxiliares (`particiona`, `merge`, `troca`...) podem existir à vontade.
- Para `(v, inicio, fim)`, a caixa **fim = n−1** escolhe se `fim` é o último índice (marcada) ou uma posição depois dele.
- Se o arquivo tiver um `main`, ele é ignorado.

## Como funciona

O vetor fica numa região de memória própria, sem permissão de acesso enquanto o sort roda. Cada leitura ou escrita faz o processador parar (`SIGSEGV`). O rastreador anota o acesso, libera só aquela instrução (Trap Flag do x86_64) e protege de novo. Por isso o código não precisa de nenhuma alteração: o `V[i] > V[i+1]` vira duas leituras na animação.

Importante saber:

- **A contagem de acessos é do código sem otimização** (`-O0` / `opt-level=0`). Ela mostra o que o seu código pede. Com otimização, o compilador guarda valores em registradores e acessa menos a memória. Para comparar velocidade, use a aba de tempo.
- **Acesso fora do vetor não derruba o programa.** Há uma página de guarda antes e depois do vetor: ler `V[N]` ou `V[-1]` aparece como aviso "acesso fora do vetor", com o índice.
- Loop infinito para no tempo limite (15 s na animação). Crash, `panic` e erro de compilação aparecem no painel da linguagem.
- O rastro guarda no máximo 4 milhões de acessos; depois disso o sort termina sem animação.

## Arquivos

- `src/detectar.rs`: lê os arquivos e descobre a função e a assinatura.
- `src/runner.rs`: compila e executa os runners, e lê a saída.
- `src/app.rs`: a janela (egui).
- `runtime/vt_trace.c`: o rastreador e o gerador de entradas (o mesmo para as duas linguagens).
- `runtime/runner_c.c`, `runtime/runner_rs.rs`: o programa que chama o sort, em cada linguagem.
- Os binários gerados ficam em `target/vt/` e são reaproveitados enquanto o arquivo do sort não mudar.
