# Regras de áudio em tempo real

> 🇬🇧 [Read in English](../REALTIME-AUDIO-RULES.md)

O callback de áudio não é código ordinário. Ele roda em uma thread com prazo
limite exigido pelo sistema operacional, e a consequência de perder esse prazo
não é um programa lento — é um clique audível, ou silêncio.

A 48 kHz com buffer de 128 amostras o callback é invocado a cada **2,67 ms** e
deve retornar bem dentro desse tempo, **toda e qualquer vez**. Não na média. O
percentil 99,9 é o número que importa, porque o único buffer atrasado a cada
mil é o que o ouvinte ouve.

Tudo abaixo segue dessa realidade.

## A lista de proibições

Dentro de `process` / `process_block`, ou de qualquer coisa que eles chamem:

| Proibido | Por quê |
|---|---|
| Alocar ou liberar memória | O alocador segura um lock e pode chamar o kernel. Latência ilimitada. |
| Travar um mutex | Inversão de prioridade: uma thread de baixa prioridade segurando o lock paralisa a thread de áudio indefinidamente. |
| Qualquer chamada do sistema | I/O de arquivo, sockets, `println!`, logging. Todos ilimitados. |
| `panic!`, `unwrap`, `expect`, `assert!` | Um pânico na thread de áudio aborta o processo ou desenrola através de um callback C. Ambos são piores do que qualquer amostra errada. |
| Loops ilimitados | `while !converged` não tem pior caso. Todo loop precisa de um máximo em tempo de compilação. |
| Esperar por qualquer coisa | Channels, variáveis de condição, joins, sleeps. |
| Crescer um `Vec`, `String`, `HashMap` | Alocação oculta. |
| `Box<dyn Trait>` no caminho por-amostra | Uma chamada indireta que o otimizador não consegue ver através. Ver `PERF-003`. |

## Como este projeto reforça isso

A imposição é estrutural, não uma questão de lembrar:

- **`piano-core` é `no_std` + `alloc`.** A maioria das operações proibidas nem
  são alcançáveis — não há `std::fs`, não há `std::sync::Mutex`, não há
  `println!`.
- **`#![forbid(unsafe_code)]`** em todo crate. Unsafe é opt-in por crate, com
  justificativa escrita, e hoje nenhum crate opta.
- **Clippy nega `unwrap_used`, `expect_used`, `panic` e `unimplemented`** no
  nível do workspace. Módulos de teste optam por não participar explicitamente;
  código de produção não consegue.
- **Todo construtor valida e toda função no caminho quente é total.** Parâmetros
  inválidos são rejeitados por `ParamError` na construção. `read_interpolated`
  mascara seu índice e satura `NaN` em vez de entrar em pânico. Coeficientes de
  filtro são fixados abaixo do círculo unitário para que um loop não possa
  divergir.
- **Toda alocação acontece em construtores.** `DelayLine::with_capacity` é a
  única função alocadora em `piano-core`, e está documentada como tal.
- **`panic = "abort"` no perfil de release.** Se um pânico acontecer, não vai
  desenrolar através de um callback de áudio C e corromper o host.

## Entrando e saindo dados

Mudanças de parâmetro, note-on e note-off vêm de uma thread diferente. A regra
acima proíbe travar, então o único mecanismo aceitável é um **ring buffer
lock-free produtor-único/consumidor-único** de structs de comando com dados
comuns, drenado no topo de cada callback.

```
Thread de UI / MIDI  ──push──▶  ring SPSC (capacidade fixa)  ──drain──▶  thread de áudio
```

Consequências que são fáceis de errar:

- O ring tem **capacidade fixa**. Quando está cheio, o produtor descarta ou
  bloqueia — a thread de áudio nunca espera.
- Comandos são **dados comuns `Copy`**. Não há `String`, não há `Box`, não há
  `Arc` cuja última clonagem possa ser descartada (e portanto liberada) na
  thread de áudio.
- Dados de áudio para UI (níveis, contagem de vozes) voltam pelo mesmo caminho,
  ou através de atômicos com ordenação relaxada. Nunca um `Mutex<State>`
  compartilhado.

## Regras numéricas

- **Todo filtro recursivo deve ser comprovadamente estável.** Coeficientes são
  fixados na construção, não checados em tempo de execução.
- **`NaN` é uma infecção permanente.** Uma vez que `NaN` entra em um loop de
  realimentação nunca sai. `math::clamp_or_low` mapeia `NaN` ao limite inferior
  em vez de propagá-lo — é por isso que existe e por que `f32::clamp` não é
  usado.
- **Denormais são um bug de desempenho, não de correção.** Ver `PERF-002`.
- **Output é limitado pela construção**, então um bug produz uma nota errada em
  vez de uma onda quadrada em escala total nos fones de alguém.

## O que "nunca trava" significa aqui

O requisito do usuário era código que nunca fica pendurado. Concretamente, este
projeto interpreta isso como quatro propriedades, cada uma das quais é testável:

1. **Totalidade.** Toda função no caminho quente retorna um valor para toda
   entrada, incluindo `NaN`, `±∞`, zero e `usize::MAX`. Verificado com
   `proptest`.
2. **Limitação.** A magnitude da saída permanece finita para qualquer combinação
   de parâmetro alcançável, para execuções arbitrariamente longas. Verificado
   com testes de execução longa.
3. **Determinismo.** A mesma semente e entradas produzem saída idêntica em nível
   de byte, então uma falha pode ser reproduzida.
4. **Tempo previsível.** Nenhuma operação no callback tem duração de pior caso
   ilimitada.
