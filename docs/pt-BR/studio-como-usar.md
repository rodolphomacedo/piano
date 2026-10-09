# Estúdio de parâmetros — como usar

Este guia não é numerado como M5–M8: o "parameter studio" é um recurso
separado do roteiro de milestones (`docs/ROADMAP.md`), documentado em inglês
em `docs/PARAMETER-STUDIO.md`. Este arquivo é só o "o que digitar e o que
esperar", em português, para essa funcionalidade específica.

## O que é isso, em uma frase

Uma página web local (sem instalar nada além do que você já tem) onde você
toca o piano inteiro — 88 teclas, com acordes, com ou sem teclado MIDI — e
arrasta controles deslizantes para mudar, ao vivo, qualquer parâmetro físico
de qualquer corda, tecla ou do instrumento como um todo, ouvindo a mudança na
hora, e opcionalmente salvando o resultado num arquivo.

**Não confunda com o item "4. No navegador" do
[`LEIA-ME.md`](LEIA-ME.md).** Aquele é uma demonstração em WebAssembly, com
uma corda só, sem MIDI. Este aqui é o programa de sempre (o mesmo binário do
`keyboard` e do `midi`) com um servidor web local embutido — o instrumento
inteiro, com todas as 88 teclas e todos os parâmetros.

## O erro mais comum: "No such file or directory"

O comando abaixo **exige** um arquivo `.piano.json` já existente — ele não
cria um sozinho. Se você tentar rodar com um nome de arquivo que ainda não
existe, vai ver exatamente este erro:

```
Error: could not load meu-piano.piano.json

Caused by:
    0: could not access meu-piano.piano.json: No such file or directory (os error 2)
```

Isso não é um bug — é o programa avisando corretamente que o arquivo pedido
não está lá. A solução é criar o arquivo primeiro. O menor arquivo válido
possível é este, que usa os valores físicos padrão do projeto para todo o
instrumento:

```sh
echo '{}' > meu-piano.piano.json
```

Se quiser dar um nome ao piano (aparece no título da página), use:

```sh
echo '{"name": "Meu Piano"}' > meu-piano.piano.json
```

O formato completo — com afinação por registro, grupos de cordas nomeados e
sobreposições por corda — está descrito com um exemplo comentado em
`docs/PARAMETER-STUDIO.md`, na seção "Piano file format" (em inglês; a
estrutura do JSON, porém, fala por si).

## Como rodar

Com o arquivo criado:

```sh
cargo run --release -p piano-cli -- studio --piano meu-piano.piano.json
```

Espere a compilação terminar (só na primeira vez) e você verá algo como:

```
loaded meu-piano.piano.json — 222 strings across 88 keys
piano studio listening on http://127.0.0.1:7878
open that address in a browser to play and edit live.
no MIDI controller requested — play from the browser instead.
Esc or Ctrl+C (in this terminal) to quit.
```

Abra `http://127.0.0.1:7878` (ou o endereço exatamente como impresso) no seu
navegador.

**Um detalhe importante**: rode este comando num terminal de verdade que
você vai deixar aberto — não em segundo plano (`nohup`, `&` desacompanhado,
como serviço). O programa precisa de um terminal interativo de verdade para
saber quando você aperta `Esc` ou `Ctrl+C` e encerrar de forma limpa; sem
isso, ele fecha sozinho logo depois de imprimir o endereço. Fechar a janela
do terminal também encerra o servidor — a página do navegador para de
responder, e isso é esperado, não um travamento.

## Tocando com um teclado MIDI ao mesmo tempo

Se você tem um controlador MIDI conectado, some `--midi` ao comando: o
teclado físico e a página do navegador tocam o mesmo instrumento ao mesmo
tempo, cada um vendo o que o outro faz em tempo real.

```sh
cargo run --release -p piano-cli -- studio --piano meu-piano.piano.json --midi
```

Os três pedais do teclado MIDI funcionam:

- **CC64, sustain (pedal da direita)**: o valor é usado de forma contínua.
  Com um pedal que suporta meio-pedal, pisar até a metade deixa a nota soando
  por menos tempo; pisar até o fundo segura tudo.
- **CC66, sostenuto (pedal do meio)**: segura só as teclas que estavam
  apertadas no instante em que você pisou nele.
- **CC67, una corda (pedal da esquerda)**: o martelo passa a bater em uma
  corda a menos por nota. O som fica mais baixo e mais "velado", não só mais
  fraco.

## O que dá para fazer na página

- **Tocar**: clique nas teclas do desenho do piano, ou use o teclado do
  computador (`a` até `;` na fileira de baixo, `w e t y u o p` na fileira de
  cima, seguindo o desenho de um piano de verdade). `z`/`x` descem/sobem uma
  oitava. `espaço` segura o pedal de sustain. As caixinhas "sostenuto" e
  "una corda" ligam os outros dois pedais.
- **Editar uma corda**: clique numa tecla, escolha qual das cordas daquele
  uníssono (uma, duas ou três, dependendo do registro) na abinha que
  aparece, e arraste os controles: amortecimento, sustentação,
  inarmonicidade, desafinação em cents, semente de ruído da excitação, o
  "zero" do filtro de perdas (quanto da perda cai só nas parciais mais
  agudas), o ponto onde o martelo bate na corda, quanto ruído entra na
  martelada, e os cinco parâmetros do martelo de feltro (expoente de
  contato, rigidez, massa, impedância da corda e o brilho do feltro,
  "felt brightness": mais alto é feltro duro, envernizado, com ataque
  brilhante; mais baixo é feltro agulhado, macio, com ataque abafado). A
  desafinação de cada corda já começa na desafinação natural do uníssono
  (alguns centésimos de semitom entre as cordas da mesma tecla), que é o
  que faz o som "respirar"; zerar tudo deixa o piano com som de órgão. Os controles que cobrem
  várias ordens de grandeza (rigidez, massa, impedância) andam em escala
  logarítmica: cada pedacinho do controle multiplica o valor pelo mesmo
  fator.
- **Editar várias cordas de uma vez**: acima dos controles, escolha "esta
  corda" (padrão), "tecla inteira" (as duas ou três cordas do uníssono
  daquela tecla) ou "seleção" (várias teclas escolhidas com shift-clique).
  A mudança se aplica a todas de uma vez, sempre corda por corda por baixo
  dos panos — nunca vira uma "entidade" nova dentro do arquivo.
- **Som do instrumento inteiro** (seção "sound" da página): um controle
  deslizante para cada ajuste que vale para o piano todo, cada um salvo no
  `.piano.json` dentro de `instrument`:
  - `room_mix` ("room mix"): quanto da sala o piano soa dentro. `0` é o
    piano seco, como num estúdio abafado; o padrão (`0.25`) põe uma sala
    média atrás do instrumento, como numa gravação de piano de concerto. O
    som sai em estéreo: graves à esquerda, agudos à direita, como o pianista
    ouve sentado no banco.
  - `room_size` ("room size"): o tamanho da sala. `1.0` é uma sala média;
    `0.5` é uma sala de estar, com reflexões muito próximas; `2.0` é uma sala
    de concerto, com as reflexões chegando bem mais espaçadas.
  - `room_reverb_seconds` ("room reverb (bass), s"): quantos segundos a sala
    leva para silenciar nos graves. `1.8` é uma sala de recital; `0.5` é um
    estúdio abafado; `4` ou mais é uma igreja.
  - `room_treble_reverb_seconds` ("room reverb (treble), s"): o mesmo para
    os agudos. Mais baixo deixa a sala escura, com cortinas e plateia; mais
    alto deixa a sala brilhante, com paredes duras. Numa sala real, os agudos
    morrem antes dos graves.
  - `room_predelay_milliseconds` ("room predelay, ms"): o silêncio antes da
    sala responder. Mais longo dá a sensação de paredes mais distantes e
    deixa o ataque das notas mais limpo.
  - `soundboard_mix_gain` ("soundboard mix"): o quanto da caixa de
    ressonância entra de volta na mistura.
  - `action_noise_gain` ("action noise"): o volume do baque da tecla no
    fundo do teclado e do abafador pousando na corda ao soltar.
  - `phantom_gain` ("phantom partials"): a força dos parciais fantasmas dos
    graves — o "corpo" metálico que um piano de cauda tem na região grave.
  - `duplex_gain` ("duplex ring"): o quanto os trechos livres de corda dos
    agudos (a escala duplex da Steinway) ressoam junto com a nota.
  - `damper_strength` ("damper strength"): a força com que o abafador de
    feltro segura a corda quando você solta a tecla. Mais alto, a nota para
    seca; mais baixo, ela ainda soa um pouco depois de soltar.
  - `velocity_curve_exponent` ("velocity curve"): `1.0` é o mapeamento
    linear original; o padrão, `1.8`, afasta o toque suave do forte porque a
    resposta do próprio martelo de feltro comprime o topo da faixa e deixa o
    fundo praticamente mudo — ver o comentário de
    `piano_audio::velocity_curve::DEFAULT_VELOCITY_CURVE_EXPONENT` para a
    medição.
  - `master_gain` ("master gain"): o volume geral de saída, aplicado logo
    antes do limitador, então abaixá-lo reduz a limitação em vez de
    alimentar um limitador já saturado.
  - `limiter_threshold` ("limiter threshold"): a partir de que volume o
    limitador de saída começa a segurar o som. Mais baixo protege acordes
    fortes de distorcer, mas achata a dinâmica; o padrão é `0.9`.
- **Caixa e ponte**: os 28 modos da caixa de ressonância (frequência, tempo
  de decaimento, ganho) e os dois ganhos de acoplamento da ponte (entre as
  cordas da mesma tecla, e entre teclas diferentes, responsável pela
  ressonância por simpatia).
- **Salvar**: digite um caminho no campo do topo e clique "Save" — grava um
  `.piano.json` novo com todo o instrumento já resolvido (nunca uma
  "diferença" em cima do que foi carregado, então o arquivo salvo sempre
  soa exatamente como está soando agora, sem depender do que veio antes).
- **Carregar**: digite um caminho de um `.piano.json` existente e clique
  "Load" — troca o instrumento inteiro, ao vivo, sem reiniciar o programa.
- **Várias abas ao mesmo tempo**: abra a mesma URL em duas abas ou dois
  computadores na mesma rede — uma mudança feita numa aba aparece na outra
  na hora.

## Se o fone ou a caixa de som for desconectado

Pode desconectar e reconectar à vontade. Em até um segundo o programa
percebe que o dispositivo de saída mudou, abre o novo (mesmo que ele use
outra taxa de amostragem, por exemplo 44100 em vez de 48000) e volta a tocar
com todas as suas edições, os pedais e a sala exatamente como estavam. A
nota que estava soando no momento da troca se perde — o piano recomeça em
silêncio, então nenhuma nota fica presa. No terminal aparece a linha
`audio device changed; playing again at ... Hz`.

## O que ainda não existe (limitações honestas)

- **Só na sua própria rede local.** O servidor escuta apenas em
  `127.0.0.1` (a própria máquina) — não existe um jeito embutido de tocar
  de outro computador pela internet, e isso é proposital: não há login nem
  senha, então abrir isso para fora seria abrir o controle do instrumento
  para qualquer pessoa.
- **Criar um grupo nomeado novo só dá pra fazer editando o arquivo.** A
  página mostra e deixa aplicar mudanças a grupos que já existem no arquivo
  carregado, e a opção "seleção" já cobre editar várias teclas de uma vez —
  mas dar nome e salvar uma seleção como um grupo reutilizável ainda exige
  editar o JSON à mão (veja a seção "groups" do exemplo em
  `docs/PARAMETER-STUDIO.md`).
- **Sem desfazer.** Cada controle desliza a mudança direto no instrumento
  que está tocando. Se errar a mão, ajuste o controle de volta, ou recarregue
  (`Load`) o último arquivo salvo.
