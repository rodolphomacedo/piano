# M5 — como usar (pedal de sustain, parar notas, e acordes)

Este guia é só para você testar o que o milestone M5 trouxe de novo. Ele não
substitui o `README.md` (em inglês) nem os documentos em `docs/` — é um
"o que digitar e o que esperar ouvir", em português.

## O que mudou nesse milestone

Até o M4, o piano tocava uma nota por vez e ela só parava de tocar sozinha,
morrendo aos poucos — não tinha como "soltar a tecla" de verdade. O M5
resolve três coisas:

1. **Parar uma nota antes dela morrer sozinha.** Agora, quando você solta
   uma tecla no teclado MIDI, a corda é "abafada" (como o feltro do abafador
   de um piano de verdade encostando na corda) e a nota some rapidamente em
   vez de continuar tocando por vários segundos.
2. **Pedal de sustain (o pedal da direita).** Se o seu teclado MIDI tem um
   pedal de sustain plugado, segurá-lo faz as notas continuarem tocando
   mesmo depois de você soltar as teclas — exatamente como um piano
   acústico. Soltar o pedal libera tudo que estava sendo segurado.
3. **Tocar vários sons ao mesmo tempo (acordes).** Cada uma das 88 teclas já
   tem sua própria "corda" reservada desde o M2, então tocar um acorde
   inteiro (várias notas juntas) já funciona sem cortar nenhuma nota.

Cada tecla do piano agora também tem um "jeito de soar" um pouco diferente
dependendo do registro: graves ficam com um brilho e um tempo de decaimento
diferentes dos agudos, em vez de todas as 88 teclas usarem exatamente o
mesmo ajuste.

## Antes de começar

Você precisa ter o projeto compilando. Se ainda não testou isso, rode a
partir da pasta do projeto:

```sh
cargo build --release
```

Isso pode demorar alguns minutos na primeira vez. Espere terminar sem erro
antes de seguir.

## Passo a passo: tocar do teclado MIDI (a forma recomendada para testar o M5)

Você precisa de um piano digital, teclado controlador MIDI ou controlador
MIDI qualquer, ligado ao computador por USB (ou por um cabo MIDI, se seu
computador tiver entrada para isso).

**1. Veja se o computador enxerga o seu teclado:**

```sh
cargo run --release -p piano-cli -- midi --list
```

Você deve ver o nome do seu equipamento aparecer numa lista. Se a lista
vier vazia, confira o cabo USB e se o teclado está ligado.

**2. Comece a tocar:**

```sh
cargo run --release -p piano-cli -- midi
```

O terminal vai mostrar uma mensagem tipo:

```
piano midi — connected to "Nome do seu teclado".
play your MIDI controller; nothing is written to disk.
CC74 (brightness) -> damping, inverted   CC1 (mod wheel) -> sustain
CC64 (sustain/hold pedal) -> holds every released note until the pedal comes back up
note-off releases a key early — release the damper instead of ringing on;
play a chord and every held note sounds together.
Esc or Ctrl+C (in this terminal) to quit.
```

Toque uma tecla no seu teclado MIDI. Você deve ouvir o som saindo pelo
alto-falante do computador, igual já acontecia antes do M5.

**3. Teste "parar a nota antes dela morrer sozinha":**

Toque uma nota grave (do lado esquerdo do teclado) e segure por um
segundo. Nas versões anteriores, ela continuaria tocando por vários
segundos mesmo depois de você soltar a tecla. Agora, no instante em que
você **solta a tecla**, o som deve cair rapidamente — em menos de meio
segundo — em vez de continuar ecoando.

Se você não ouvir diferença nenhuma entre segurar e soltar, algo não está
funcionando como esperado; isso é o ponto principal deste milestone.

**4. Teste o pedal de sustain (só funciona se você tiver o pedal físico
ligado ao teclado MIDI):**

- Pise e segure o pedal de sustain.
- Toque uma nota e solte a tecla rapidamente.
- Mesmo com a tecla solta, a nota deve continuar tocando enquanto o pedal
  estiver pressionado — ela não deve morrer rápido como no passo 3.
- Solte o pedal. Nesse instante, a nota (e qualquer outra que você tenha
  tocado e soltado enquanto o pedal estava pressionado) deve parar
  rapidamente, todas de uma vez.

Se o seu teclado MIDI **não tem pedal físico**, pule este passo — não tem
como testar sem o pedal conectado.

**5. Teste acordes:**

Toque três ou quatro teclas ao mesmo tempo (por exemplo, um acorde de Dó
maior: C, E e G). Você deve ouvir todas as notas tocando juntas,
misturadas, sem nenhuma cortar a outra.

Para sair, aperte `Esc` ou `Ctrl+C` com o terminal em foco.

## Passo a passo: tocar do teclado do computador

```sh
cargo run --release -p piano-cli -- keyboard
```

Isso funciona como já funcionava antes: a fileira de baixo do teclado
(`Z S X D C V G B H N J M ,`) toca uma oitava, a fileira de cima continua
subindo. `[` e `]` mudam o "damping" (brilho), `-` e `=` mudam o sustain.

**Aviso importante e honesto sobre esta forma de tocar:** a maioria dos
terminais — incluindo o Terminal.app padrão do macOS — **não consegue
detectar quando você solta uma tecla do computador**, só quando você
aperta. Por isso, tocando pelo teclado do computador, a nota **sempre vai
morrer sozinha**, do jeito que já era antes do M5; soltar a tecla não
antecipa o fim da nota. Só alguns terminais bem específicos (como o
`kitty` ou o `WezTerm`) conseguem reportar isso — se você usar um deles, o
programa detecta sozinho e avisa na tela; se detectar, soltar a tecla vai
abafar a nota mais cedo, igual ao MIDI. Para ter o controle real de "soltar
a tecla e a nota morre na hora", use o teclado MIDI (passo a passo acima) —
essa é, hoje, a única forma garantida de conseguir isso.

## Resumo do que esperar ouvir

| O que você faz | O que deve acontecer |
|---|---|
| Tocar uma tecla MIDI | Som imediato, igual antes |
| Soltar a tecla MIDI | Som cai rapidamente (menos de meio segundo) |
| Segurar o pedal de sustain e soltar a tecla | Som continua tocando |
| Soltar o pedal de sustain | Tudo que estava "pendurado" no pedal para de tocar |
| Tocar várias teclas juntas | Todas tocam ao mesmo tempo, sem cortar |
| Soltar uma tecla do teclado do computador | Nota continua tocando até morrer sozinha (quase sempre) |

Se alguma dessas linhas não bater com o que você ouviu, vale registrar
como um problema — esse é exatamente o comportamento que este milestone
deveria ter entregado.
