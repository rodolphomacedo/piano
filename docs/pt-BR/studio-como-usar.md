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

## O que dá para fazer na página

- **Tocar**: clique nas teclas do desenho do piano, ou use o teclado do
  computador (`a` até `;` na fileira de baixo, `w e t y u o p` na fileira de
  cima, seguindo o desenho de um piano de verdade). `z`/`x` descem/sobem uma
  oitava. `espaço` segura o pedal de sustain.
- **Editar uma corda**: clique numa tecla, escolha qual das cordas daquele
  uníssono (uma, duas ou três, dependendo do registro) na abinha que
  aparece, e arraste os controles: amortecimento, sustentação,
  inarmonicidade, desafinação em cents, semente de ruído da excitação, e os
  três parâmetros do martelo de feltro (expoente de contato, rigidez,
  massa).
- **Editar várias cordas de uma vez**: acima dos controles, escolha "esta
  corda" (padrão), "tecla inteira" (as duas ou três cordas do uníssono
  daquela tecla) ou "seleção" (várias teclas escolhidas com shift-clique).
  A mudança se aplica a todas de uma vez, sempre corda por corda por baixo
  dos panos — nunca vira uma "entidade" nova dentro do arquivo.
- **Editar o instrumento inteiro**: os 8 modos da caixa de ressonância
  (frequência, tempo de decaimento, ganho) e os dois ganhos de acoplamento
  da ponte (entre as cordas da mesma tecla, e entre teclas diferentes,
  responsável pela ressonância por simpatia).
- **Salvar**: digite um caminho no campo do topo e clique "Save" — grava um
  `.piano.json` novo com todo o instrumento já resolvido (nunca uma
  "diferença" em cima do que foi carregado, então o arquivo salvo sempre
  soa exatamente como está soando agora, sem depender do que veio antes).
- **Carregar**: digite um caminho de um `.piano.json` existente e clique
  "Load" — troca o instrumento inteiro, ao vivo, sem reiniciar o programa.
- **Várias abas ao mesmo tempo**: abra a mesma URL em duas abas ou dois
  computadores na mesma rede — uma mudança feita numa aba aparece na outra
  na hora.

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
