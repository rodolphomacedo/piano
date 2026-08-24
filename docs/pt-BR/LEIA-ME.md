# Piano — guia completo em português

Desenvolvimento patrocinado pela [Grabatus Labs](https://grabatus.com).

Este é o ponto de partida em português para todo o projeto: o que ele é, por
que existe, como instalar, e como usar cada uma das formas de tocar. Os
outros arquivos em `docs/pt-BR/` (`M5-como-usar.md`, `M6-como-usar.md`, etc.)
são complementares — cada um foca só no que mudou naquele milestone
específico. Este arquivo é o "leia isto primeiro".

O resto da documentação técnica do projeto (código, comentários, nomes de
commit) está em inglês, por regra do próprio projeto — isso não muda. Mas
você não precisa ler nada em inglês para instalar e tocar; este guia basta.

## O que é este projeto, em uma frase

Um piano digital que **calcula** o som de uma corda vibrando de verdade, em
vez de tocar uma gravação.

## O que isso quer dizer, sem jargão

A maioria dos "pianos digitais" que existem por aí — teclados eletrônicos,
plugins de computador, aplicativos de celular — funciona gravando um piano
de verdade nota por nota e reproduzindo essas gravações depois (isso se
chama "sampler"). Funciona bem, mas tem limites: você só tem exatamente as
notas e intensidades que alguém gravou, o arquivo é pesado, e não existe
nenhuma "física" ali dentro — é só tocar um áudio.

Este projeto faz outra coisa: ele tem, dentro do código, uma simulação
matemática de como uma corda de piano real se comporta quando é golpeada por
um martelo de feltro — a rigidez da corda, o jeito como ela perde energia ao
longo do tempo, como duas ou três cordas da mesma tecla batem entre si, como
a caixa de ressonância do piano colore o som. O som que você ouve é
*calculado*, amostra por amostra, 48 mil vezes por segundo, a partir dessa
física — não é gravação de ninguém tocando. Por isso o projeto nunca vai ter
um arquivo de áudio dentro dele: não precisa, e a regra do projeto proíbe
isso de propósito.

A vantagem prática: qualquer nota, com qualquer intensidade, em qualquer
combinação, soa "de verdade" — porque nasceu da mesma física de sempre, não
de uma gravação específica que alguém precisou fazer com antecedência.

## Isso já funciona? Eu posso usar agora?

**Sim.** O projeto passou por 8 etapas de desenvolvimento (chamadas de
"milestones", M1 a M8) e todas estão prontas, testadas e verificadas — não é
um protótipo incompleto. Hoje você já pode:

- Tocar pelo teclado do computador, sem precisar de nenhum equipamento extra.
- Conectar um piano digital ou teclado MIDI de verdade por USB e tocar nele.
- Renderizar uma nota para um arquivo de áudio (`.wav`).
- Abrir uma versão simplificada direto no navegador, sem instalar nada.

Nada disso está "quase pronto" ou "em teste" — os quatro caminhos acima
funcionam de verdade, hoje, na versão atual do código.

## Antes de instalar: o que você precisa

Só uma coisa: o **Rust** instalado, através da ferramenta oficial chamada
`rustup`. Se você nunca instalou Rust neste computador, rode isto em um
terminal:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Siga as instruções que aparecerem na tela (a opção padrão serve). Depois
disso, feche e abra o terminal de novo antes de continuar.

## Instalando o projeto

```sh
git clone https://github.com/rodolphomacedo/piano.git
cd piano
```

Isso copia o projeto inteiro para o seu computador. Não precisa compilar nada
separadamente — o comando que toca o piano (abaixo) compila tudo sozinho na
primeira vez que você roda, o que pode levar alguns minutos. Da segunda vez
em diante é rápido.

## Como tocar: as quatro formas

### 1. Pelo teclado do próprio computador (a forma mais simples)

```sh
cargo run --release -p piano-cli -- keyboard
```

Espere a mensagem de "compilando" terminar (só na primeira vez) e comece a
digitar. As teclas do seu teclado viram teclas de piano:

- Fileira de baixo (`Z S X D C V G B H N J M ,`) — uma oitava.
- Fileira de cima (`Q 2 W 3 E R 5 T 6 Y 7 U I 9 O 0 P`) — continua para cima.

Segure várias teclas ao mesmo tempo para tocar um acorde. `[` e `]` mudam o
quanto a nota "morre" rápido (amortecimento); `-` e `=` mudam o quanto ela
sustenta. `Esc` ou `Ctrl+C` para sair.

**Um detalhe honesto**: a maioria dos terminais (incluindo o Terminal padrão
do macOS) não avisa o programa quando você *solta* uma tecla — só quando
aperta. Por isso, no terminal padrão, as notas tocam e vão "morrendo"
sozinhas com o tempo, mesmo depois que você já soltou a tecla — isso é uma
limitação do terminal, não do piano. Terminais mais modernos como o `kitty`
ou o `WezTerm` já resolvem isso automaticamente, sem configuração.

### 2. Com um piano digital ou teclado MIDI de verdade

Se você tem um teclado musical conectado por USB ou cabo MIDI:

```sh
cargo run --release -p piano-cli -- midi --list   # mostra o que está conectado
cargo run --release -p piano-cli -- midi           # começa a tocar
```

Toque normalmente — as notas soam ao pressionar e param ao soltar, de
verdade (diferente do teclado do computador, aqui não tem a limitação
acima). Se o seu teclado tiver um pedal de sustain (o pedal da direita),
segurá-lo mantém as notas soando mesmo depois de soltar as teclas, como num
piano acústico de verdade — inclusive fazendo o resto do instrumento
ressoar levemente junto, por simpatia. `Esc` ou `Ctrl+C` no terminal para
sair.

### 3. Gerando um arquivo de áudio

Se você só quer um arquivo `.wav` de uma nota, sem tocar ao vivo:

```sh
cargo run --release -p piano-cli -- render --note A4 --seconds 3 --output minha-nota.wav
```

Troque `A4` pela nota que quiser (por exemplo `C3`, `F#5`) e `minha-nota.wav`
pelo nome do arquivo. O arquivo aparece na pasta onde você rodou o comando.

### 4. No navegador, sem instalar nada além do Rust

Esta versão roda direto numa aba do navegador, mas é mais simples que as
outras: só uma corda de cada vez, sem MIDI, sem acordes. Serve para
demonstrar a tecnologia rodando fora do computador que a compilou.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.100 --locked
cargo build -p piano-wasm --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir crates/piano-wasm/www/pkg target/wasm32-unknown-unknown/release/piano_wasm.wasm
cd crates/piano-wasm/www && python3 -m http.server 8080
```

Depois abra `http://localhost:8080` no navegador, mexa no controle de
frequência e clique em "Strike" (golpear).

## Por que o projeto é construído do jeito que é

Três ideias guiam basicamente toda decisão técnica deste projeto:

1. **Tem que ser eficiente.** Um piano de verdade pode ter até 240 cordas
   soando ao mesmo tempo, e o som precisa ser calculado em tempo real, sem
   atraso perceptível — cada pedacinho de som tem só 2,67 milissegundos para
   ser calculado antes de precisar tocar.
2. **Nunca pode travar.** Um sintetizador que trava no meio de uma música é
   inaceitável — então o código que calcula o som, especificamente, nunca
   aloca memória nova, nunca espera por outra parte do programa, e nunca
   pode gerar um erro fatal enquanto está tocando. Isso é garantido pela
   estrutura do código, não apenas por cuidado dos programadores.
3. **O motor de física não sabe onde está rodando.** O mesmo código que
   calcula a vibração da corda funciona igual rodando no computador, no
   navegador, ou (no futuro) dentro de um plugin de estúdio — sem saber a
   diferença.

## O que ainda não existe (e por quê)

- **`cargo install piano-cli`** (instalar com um comando só, sem clonar o
  repositório) ainda não funciona — os pacotes do projeto estão prontos
  para serem publicados no repositório oficial de pacotes Rust
  (crates.io), mas essa publicação é uma decisão deliberada que ainda não
  foi tomada, não um trabalho pendente. Enquanto isso não acontece, o
  caminho "Instalando o projeto" acima é o que funciona.
- **Um plugin de estúdio** (para usar dentro de programas como Ableton,
  Logic ou Reaper) foi pesquisado e documentado (`docs/PLUGIN-PATH.md`, em
  inglês), mas ainda não foi construído — a conclusão da pesquisa foi
  "ainda não vale a pena, mas o caminho já está mapeado".

## Onde encontrar mais

- **Quer entender a física e a matemática por trás do piano, do zero, sem
  pré-requisito nenhum?** Leia
  [`docs/pt-BR/COMO-FUNCIONA.md`](COMO-FUNCIONA.md) — uma aula completa, em
  português, explicando cada conceito de música e de física antes de usá-lo.
  Também disponível como PDF para baixar:
  [`docs/pt-BR/COMO-FUNCIONA.pdf`](COMO-FUNCIONA.pdf).
- Cada etapa do projeto (M5 a M8) tem seu próprio guia de "o que mudou e
  como testar" em `docs/pt-BR/` — bom para quem quer acompanhar a evolução
  passo a passo.
- Toda a documentação técnica detalhada (arquitetura, física, performance)
  está em inglês na pasta `docs/` e é referenciada pelo `README.md`
  principal, na raiz do projeto — não é necessária para usar o piano, só
  para quem quiser entender ou mexer no código.
- O projeto é 100% código aberto, licenciado sob MIT ou Apache-2.0 (você
  escolhe qual das duas usar) — pode ser copiado, modificado e redistribuído
  livremente, inclusive comercialmente, respeitando os termos dessas
  licenças.
