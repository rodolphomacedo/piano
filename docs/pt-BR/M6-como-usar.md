# M6 — como usar (cordas em uníssono, ressonância simpática, soundboard)

Este guia é só para você testar o que o milestone M6 trouxe de novo. Ele não
substitui o `README.md` (em inglês) nem os documentos em `docs/` — é um
"o que digitar e o que esperar ouvir", em português.

## O que o M6 trouxe

Até o M5, cada tecla do piano tocava com **uma única corda**. Um piano de
verdade não funciona assim: a maioria das teclas é tocada por 2 ou 3 cordas
físicas, afinadas quase, mas não exatamente, na mesma frequência. O M6
resolve três coisas, todas ligadas entre si:

1. **Cordas em uníssono e batimento.** Cada tecla agora tem o número de
   cordas de um piano de verdade: 1 corda nos graves (as 12 teclas mais
   graves), 2 cordas na região do tenor (as próximas 18 teclas) e 3 cordas
   no resto do agudo (as 58 teclas restantes). Como as cordas de uma mesma
   tecla estão levemente desafinadas entre si (poucos "cents", uma fração de
   semitom), elas produzem um leve "batimento" — um tremor na altura/volume
   do som — e um **decaimento em duas fases**: a nota cai rápido nos
   primeiros instantes (as cordas ainda "brigando" entre si) e depois se
   assenta numa cauda mais lenta e estável.
2. **Ressonância simpática pelo pedal de sustain.** Todas as cordas do piano
   agora estão ligadas por um "barramento" compartilhado, imitando o
   cavalete (a peça de madeira onde todas as cordas realmente se conectam
   num piano de verdade). Com o pedal de sustain pressionado, os abafadores
   de **todas** as teclas sobem — não só da tecla que você está segurando —
   então tocar uma nota faz o resto do instrumento vibrar levemente junto,
   por simpatia, mesmo em cordas que você não tocou.
3. **Soundboard (a tábua harmônica).** O som de cada nota agora passa por um
   banco de ressonadores que imita, de forma simplificada, a tábua de
   madeira que amplifica e "colore" o som de um piano de verdade — em vez de
   você ouvir só a corda "crua".

## Uma limitação honesta, antes de tudo

O soundboard deste projeto é uma **aproximação paramétrica** — um punhado de
frequências de ressonância escolhidas com base na literatura sobre acústica
de pianos, não uma cópia do soundboard de um piano específico e real. Isso é
uma escolha deliberada: o projeto tem uma regra absoluta de **nunca**
incluir gravações ou amostras de áudio reais no código (nem para modelar o
soundboard) — então, em vez de gravar e reproduzir o som de um piano de
verdade, o projeto *calcula* uma aproximação razoável a partir de números
publicados em artigos científicos. O resultado é um "jeito" de soundboard
plausível, não uma réplica fiel de nenhum piano específico.

Da mesma forma, os "cents" de desafinação entre as cordas de uma mesma tecla
e a força da ressonância simpática são valores escolhidos de forma
raciocinada (dentro da faixa que a literatura descreve), não medidos de um
piano real.

## Antes de começar

Você precisa ter o projeto compilando. Se ainda não testou isso, rode a
partir da pasta do projeto:

```sh
cargo build --release
```

Isso pode demorar alguns minutos na primeira vez. Espere terminar sem erro
antes de seguir.

## Passo a passo: ouvir o batimento e o decaimento em duas fases

A forma mais fácil de notar isso é tocar uma nota isolada e prestar atenção
nos primeiros 1-2 segundos dela.

**1. Toque uma nota pelo teclado do computador:**

```sh
cargo run --release -p piano-cli -- keyboard
```

**2. Toque uma tecla na região média ou aguda do teclado** (por exemplo, a
tecla `J`, perto do meio) **e escute com atenção**, sem tocar mais nada:

- Nos primeiros instantes, o som deve parecer um pouco "mais cheio" ou
  levemente trêmulo, comparado a um tom puro e estável — isso é o batimento
  das 2 ou 3 cordas daquela tecla batendo entre si.
- Depois de mais ou menos meio segundo a um segundo, o som deve "assentar"
  numa cauda mais estável e decair de forma mais lenta e uniforme daí em
  diante.

Esse efeito é sutil — não espere um trêmolo óbvio e exagerado. Se quiser uma
comparação mais nítida, toque uma nota bem grave (por exemplo, a tecla `Z`,
a mais grave do teclado do computador): como as teclas mais graves têm
**uma única corda**, elas não têm batimento nenhum para comparar — o
contraste entre "grave sem batimento" e "média/aguda com um leve batimento"
é o jeito mais confiável de notar a diferença.

## Passo a passo: ouvir a ressonância simpática pelo pedal

Este teste precisa de um teclado MIDI com pedal de sustain físico
conectado — sem o pedal físico não tem como testar essa parte.

**1. Veja se o computador enxerga o seu teclado:**

```sh
cargo run --release -p piano-cli -- midi --list
```

**2. Comece a tocar:**

```sh
cargo run --release -p piano-cli -- midi
```

**3. Pise e segure o pedal de sustain.**

**4. Toque uma nota só, com força (staccato), e depois pare de tocar
completamente — mas continue segurando o pedal.**

Com atenção e num ambiente silencioso, você deve conseguir perceber um
leve som residual "ressoando" além da nota que você tocou — o resto do
instrumento reagindo por simpatia através do cavalete compartilhado. Esse
efeito é sutil por natureza (é assim mesmo num piano de verdade) — não é um
eco óbvio nem uma segunda nota clara, é mais uma "névoa" ou um leve
enriquecimento do som comparado a tocar a mesma nota com o pedal solto.

**5. Solte o pedal e toque a mesma nota de novo, sem pisar no pedal desta
vez, para comparar.** O som deve ficar mais "seco" e isolado, sem aquela
névoa de fundo.

Se você não notar diferença nenhuma entre os dois casos, tente um ambiente
mais silencioso e um volume mais alto — o efeito é propositalmente sutil,
não um exagero.

## O que esperar em cada passo — resumo

| O que você faz | O que deve acontecer |
|---|---|
| Tocar uma nota grave (1 corda) | Decaimento simples, sem batimento perceptível |
| Tocar uma nota média/aguda (2-3 cordas) | Leve batimento/tremor nos primeiros instantes, depois decaimento mais estável |
| Segurar o pedal, tocar uma nota, soltar as mãos | Uma leve "névoa" de ressonância do resto do instrumento, além da nota tocada |
| Tocar a mesma nota sem o pedal | Som mais seco, sem a névoa |
| Qualquer nota tocada | Timbre passa por um soundboard simulado — mais "cheio"/"encaixado" do que uma corda crua |

Se alguma dessas linhas não bater com o que você ouviu, vale registrar como
um problema — mas lembre-se de que todos esses efeitos são sutis por
natureza (o objetivo nunca foi um exagero óbvio, e sim um comportamento
fisicamente honesto de um piano real).
