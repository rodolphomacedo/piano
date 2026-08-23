# M7 — como usar (engenharia de performance)

Este guia é só para você entender o que o milestone M7 trouxe de novo. Ele
não substitui o `README.md` (em inglês) nem os documentos em `docs/` — é um
"o que digitar e o que esperar", em português.

## O aviso mais importante deste guia

**Ao contrário dos milestones anteriores (M1 a M6), o M7 não muda o som do
piano.** Se você tocar uma nota antes e depois deste milestone, ela deve
soar exatamente igual. M7 é uma parada dedicada a *medir* o quão rápido o
programa é por dentro — nenhum efeito novo, nenhuma tecla nova, nada para
ouvir de diferente. Se você esperava algo audível, este é o milestone
"errado" para procurar isso; volte ao guia do M6.

O que o M7 entrega é invisível por natureza: números que provam que o
programa continua rápido o suficiente para tocar ao vivo sem engasgar,
mesmo com o piano inteiro (88 teclas, até 222 cordas) tocando ao mesmo
tempo — e, em alguns casos, a descoberta honesta de que uma otimização
proposta não valia a pena aplicar.

## Por que isso importa mesmo sem ser audível

Um sintetizador que toca ao vivo tem um prazo rígido: a cada fração de
segundo (no caso deste projeto, a cada 128 amostras de áudio, cerca de
2,7 milésimos de segundo a 48.000 Hz), o programa precisa terminar de
calcular o próximo pedaço de som **antes** da placa de som pedir o
próximo. Se ele atrasar uma única vez, você ouve um "clique" ou um
"estalo" — um furo no áudio. O M7 existe para provar, com números reais
medidos neste computador, que isso não vai acontecer mesmo no pior caso
(as 88 teclas tocando juntas, pedal pisado, tudo ressoando).

## Um vocabulário rápido, para quem nunca viu isso

Nenhum destes termos é exigido para *usar* o piano — só para entender os
números que aparecem nos comandos abaixo, se você tiver curiosidade.

- **Benchmark ("bancada de teste de velocidade")**: um programinha que
  roda um pedaço de código várias vezes e cronometra quanto tempo cada
  execução leva, para dar um número confiável (não só "pareceu rápido").
- **Microssegundo (µs) e milissegundo (ms)**: 1 milissegundo = 1.000
  microssegundos = um milésimo de segundo. Os números abaixo estão
  nessa escala — o programa inteiro processando 88 teclas de uma vez
  ainda leva menos de 1,5 milissegundo, bem dentro do prazo de 2,67 ms.
- **Percentil (p50, p95, p99, p99.9)**: se você cronometrar 1.000
  execuções e ordenar os tempos do mais rápido ao mais lento, o "p50" é o
  tempo que fica bem no meio (metade foi mais rápida, metade mais lenta —
  a "mediana"), e o "p99.9" é o tempo tal que 999 em cada 1.000 execuções
  foram mais rápidas que ele. Por que isso importa: a **média** de 1.000
  execuções pode esconder completamente aquela vez rara em que o programa
  demorou muito mais que o normal — e é justamente essa vez rara que você
  ouviria como um clique. O p99.9 é o número que realmente prevê se o
  ouvinte vai perceber um problema.
- **"Cache" do processador**: uma memória pequena e muito rápida dentro do
  próprio processador. Quando o programa usa dados que já estão nessa
  memória rápida, ele voa; quando precisa buscar dados fora dela (na
  memória RAM comum), ele fica mais lento. Uma das perguntas deste
  milestone era: "a ordem em que processamos as 222 cordas do piano faz
  essa memória rápida ser mais bem aproveitada?" (resposta: um pouco sim,
  mas menos do que se temia — veja abaixo).

## O que foi medido, em linguagem simples

1. **"Os testes de segurança de acesso à memória custam alguma coisa?"**
   Toda vez que o programa lê a "linha de atraso" de uma corda (a memória
   que representa a corda vibrando), ele confere se está lendo no lugar
   certo. Essa conferência *poderia* estar desperdiçando tempo. Medido: a
   diferença é de menos de um quarto de ciclo do processador por leitura —
   irrelevante. **Nada foi mudado no código de produção por causa disso.**

2. **"A ordem em que processamos as cordas importa?"** Processar uma corda
   inteira antes de passar para a próxima (a ordem que o programa já usava)
   é cerca de 5% mais rápido do que processar todas as cordas uma amostra
   de cada vez. Real, mas mais modesto do que se imaginava — o "estoque"
   de dados das 222 cordas cabe na memória rápida (cache) deste
   processador. **A ordem que já existia foi mantida, agora com um número
   real por trás da decisão.**

3. **"As cordas graves, que tocam por até 40 segundos, perdem precisão
   numérica no final e ficam presas num ruído baixinho em vez de sumir de
   verdade?"** Foi gravada uma nota A0 (a mais grave do piano) por 60
   segundos inteiros e medido o volume a cada segundo. Resultado: o som
   continua diminuindo suavemente do início ao fim, sem travar em um
   volume mínimo constante — não é um problema real neste projeto.

4. **Cada peça do motor de síntese (a dispersão que faz o piano soar
   "esticado" em vez de uma corda de violão, o modelo do martelo, o
   barramento de ressonância simpática, a tábua harmônica simulada) agora
   tem seu próprio número de custo medido isoladamente**, em vez de só um
   número agregado do motor inteiro. Isso ajuda quem for otimizar no
   futuro a saber exatamente onde o tempo está sendo gasto.

5. **Uma otimização real foi encontrada, mas não aplicada — de propósito.**
   Reorganizar como as 2-3 cordas de uma mesma tecla são processadas juntas
   mostrou uma redução real de cerca de 41% no custo dessa parte específica
   do cálculo. Mas aplicar isso de verdade exigiria mexer em uma parte do
   código que todos os testes já existentes desde o M4 dependem, por um
   ganho que hoje não é o gargalo principal. A decisão, documentada
   honestamente em `docs/PERFORMANCE.md`, foi **não aplicar agora** — só
   registrar que a oportunidade existe, para um milestone futuro decidir
   se vale a pena.

## Como rodar os números você mesmo, se tiver curiosidade

Nada disso é necessário para tocar o piano normalmente — é só para quem
quiser ver os números com os próprios olhos, no seu próprio computador
(os números vão variar de máquina para máquina).

**1. As bancadas de teste de velocidade por componente** (dispersão,
martelo, barramento de ressonância, tábua harmônica, uma corda sozinha,
o teclado inteiro):

```sh
cargo bench -p piano-core
```

Isso demora alguns minutos e imprime uma lista de tempos medidos, um por
componente.

**2. O número "o motor inteiro aguenta 88 teclas tocando ao mesmo tempo
dentro do prazo?"**, incluindo a distribuição p50 a p99.9:

```sh
cargo test --release -p piano-audio -- --ignored --nocapture
```

Você deve ver uma linha parecida com:

```
88-voice callback timing over 1000 blocks: p50=950 us, p95=1000 us, p99=1100 us, p99.9=1500 us, max=1617 us (budget 2 670 us at 48 kHz)
```

Isso quer dizer: processar um bloco de áudio com as 88 teclas tocando
levou, na pior das 1.000 vezes medidas, 1.617 microssegundos (1,617
milissegundos) — bem menos que os 2.670 microssegundos (2,67 ms) de prazo
disponível. Sobra folga.

## O que esperar em cada passo — resumo

| O que você faz | O que deve acontecer |
|---|---|
| Tocar qualquer nota, como antes | Soa exatamente igual ao M6 — nada mudou no áudio |
| `cargo bench -p piano-core` | Uma lista de tempos medidos por componente, em microssegundos |
| `cargo test --release -p piano-audio -- --ignored --nocapture` | Uma linha com p50/p95/p99/p99.9/máximo, todos bem abaixo do prazo de 2,67 ms |
| Segurar o pedal, tocar um acorde grande | Continua funcionando como no M6 — o M7 não mudou esse comportamento |

Se algo aqui soar diferente do M6, ou se os números impressos excederem o
prazo de 2,67 ms na sua máquina, vale registrar como um problema — mas
lembre-se de que os números acima foram medidos em uma máquina específica
(um Intel Core i5-8259U de 2,3 GHz) e podem variar em outro computador,
principalmente um mais lento ou mais rápido.
