# M8 — como usar (empacotamento e lançamento)

Este é o último guia desta série (M4 a M8). Ele não substitui o `README.md`
(em inglês) nem os documentos em `docs/` — é um "o que digitar e o que
esperar", em português.

## O aviso mais importante deste guia

**Assim como o M7, o M8 não muda o som do piano.** Nenhuma nota vai soar
diferente. M8 é sobre *empacotar e distribuir* o que já existe desde o M7 —
compilar o programa para três sistemas operacionais diferentes de forma
repetível, deixar os pacotes prontos para publicação no repositório oficial
de bibliotecas Rust (o "crates.io"), e escrever um documento honesto sobre
se vale a pena transformar isto num plugin de áudio (resposta curta: ainda
não, mas o caminho está mapeado).

## A decisão mais importante deste milestone: o que NÃO foi feito de propósito

Existem duas ações neste mundo que, uma vez feitas, **não têm volta fácil**:

1. **Publicar de verdade no crates.io.** Depois que uma versão de um pacote
   Rust é publicada lá, ela não pode ser apagada — só pode ser marcada como
   "não use mais" (`yank`), mas continua visível e baixável para sempre.
2. **Criar um "Release" público no GitHub.** No instante em que existe, é
   público — qualquer pessoa pode ver e baixar.

Nenhuma das duas foi feita neste milestone, e isso foi uma decisão
deliberada, não um esquecimento. As duas ficaram **prontas para acontecer**
— um comando só, já testado — mas quem aperta o botão de verdade é você, o
dono do projeto, no momento que você escolher. Ninguém além de você deveria
tomar essa decisão por você, especialmente para algo que não tem "desfazer".

## O que foi de fato construído e verificado

### 1. Compilação repetível para três sistemas

Existe agora um workflow no GitHub Actions
(`.github/workflows/release-build.yml`) que compila o `piano-cli` para:

- macOS com processador Intel (`x86_64-apple-darwin`)
- macOS com processador Apple Silicon — M1/M2/M3/M4 (`aarch64-apple-darwin`)
- Linux (`x86_64-unknown-linux-gnu`)

Ele **não roda sozinho a cada `git push`** — só quando alguém pede
explicitamente ("workflow_dispatch", o botão "Run workflow" na aba Actions
do GitHub, ou `gh workflow run release-build.yml` pelo terminal). Isso foi
proposital: rodar uma compilação em três sistemas diferentes a cada commit
seria lento e caro para pouco benefício.

Este workflow foi realmente disparado e observado (não é só um arquivo YAML
escrito e nunca testado): duas das três pernas terminaram em menos de um
minuto cada (Linux e macOS Apple Silicon) e ficam disponíveis por 14 dias na
aba **Actions** do repositório no GitHub, como "artefato de workflow" — um
arquivo privado do próprio repositório, visível para quem tem acesso a ele,
bem diferente de um "Release" público. A terceira perna (macOS Intel) ficou
mais de uma hora **na fila** do próprio GitHub sem sequer começar a rodar —
isso é falta de máquinas Intel disponíveis do lado do GitHub, não um
problema deste projeto ou do workflow em si; as outras duas pernas, que
rodaram em sistemas operacionais e arquiteturas diferentes, já provam que o
workflow funciona corretamente. Se você rodar de novo mais tarde, é bem
provável que a fila já tenha liberado.

### 2. Os pacotes estão prontos para publicação, mas não publicados

Cada um dos sete pacotes deste workspace (`piano-core`, `piano-params`,
`piano-render`, `piano-audio`, `piano-midi`, `piano-wasm`, `piano-cli`) foi
testado com `cargo publish --dry-run` — um "simulado", que conversa de
verdade com o servidor real do crates.io e verifica tudo (descrição,
licença, arquivos incluídos, dependências) **sem de fato publicar nada**.

Dois pacotes passaram no teste completo, do começo ao fim:
`piano-core` (o motor de física, sem depender de nenhum outro pacote deste
projeto) e `piano-midi` (só depende de bibliotecas externas). Os outros
cinco esbarraram num limite conhecido e sem solução por fora: eles dependem
de `piano-core` (ou de `piano-audio`, no caso do `piano-cli`), e como
`piano-core` ainda não está publicado de verdade, o Cargo não consegue
"achar" essa dependência no índice do crates.io — é uma limitação estrutural
do próprio Cargo para pacotes que dependem uns dos outros dentro do mesmo
projeto, não um problema deste código. O jeito de resolver isso é publicar
na ordem certa, de verdade: primeiro `piano-core`, depois `piano-params`,
depois os outros. No momento em que `piano-core` existir de verdade lá, o
próximo teste de `piano-params` passaria sozinho.

### 3. "Alguém que não é o autor consegue instalar e tocar"

Essa era a régua de sucesso do M8, escrita no próprio roteiro do projeto
(`docs/ROADMAP.md`). Hoje, isso significa uma destas três coisas — e o
`README.md` (em inglês, mas a estrutura é a mesma) agora explica as três com
honestidade:

1. **Compilar a partir do código-fonte** — funciona hoje, exige só o
   `rustup` instalado, é o caminho "de sempre" que os guias M1 a M7 já
   descreviam.
2. **Baixar um binário já compilado** — funciona hoje, *se* alguém já
   disparou o workflow do item 1 recentemente (os artefatos expiram em 14
   dias).
3. **`cargo install piano-cli`** — só vai funcionar **depois** que você
   decidir publicar de verdade. Hoje esse comando não faz nada, porque o
   pacote não existe lá ainda.

### 4. O documento sobre plugins de áudio

`docs/PLUGIN-PATH.md` (em inglês) responde a uma pergunta que o roteiro do
projeto deixou em aberto de propósito: "vale a pena transformar isto num
plugin de áudio (o tipo de coisa que você carrega dentro de um programa de
gravação, como Ableton, Logic ou Reaper)?"

Resposta curta, em português: **ainda não, mas o formato certo já está
identificado.** Existem três formatos de plugin considerados:

- **CLAP** — um formato novo, aberto, com licença compatível com este
  projeto (MIT), funciona nos três sistemas operacionais. É o candidato
  certo, o dia que alguém decidir construir isso.
- **VST3** — tecnicamente viável, mas a licença oficial da Steinberg
  entra em conflito com a licença deste projeto (teria que ser GPL numa
  parte do código, ou pagar por uma licença comercial) — nenhuma das duas
  opções combina com este projeto sem uma decisão separada e deliberada.
- **AU (Audio Unit)** — só funciona em macOS/iOS, o que vai contra o
  objetivo deste próprio milestone de rodar nos três sistemas.

O motivo de não construir isso agora não é "é difícil" — na verdade, boa
parte do trabalho de verificação de tempo real que este projeto já faz
(nunca alocar memória, nunca travar, nunca dar pane dentro do processamento
de áudio) é *exatamente* o que um plugin também exige, então o terreno já
está preparado. O motivo real é que não havia, no ambiente onde este
milestone foi construído, um programa hospedeiro de plugins (um DAW) para
testar de verdade — e este projeto já tem a regra de só fechar um item
quando existe uma medição ou verificação real por trás, não uma suposição.

## Como conferir os números você mesmo, se tiver curiosidade

Nada disso é necessário para tocar o piano normalmente.

**Ver se o workflow de compilação existe e o que ele produziu por último:**

```sh
gh run list --workflow=release-build.yml
```

**Disparar uma nova rodada de compilação você mesmo** (precisa do `gh`
instalado e autenticado, e leva alguns minutos):

```sh
gh workflow run release-build.yml
gh run watch   # acompanha até terminar
```

**Simular uma publicação sem publicar de verdade**, para qualquer pacote:

```sh
cargo publish --dry-run -p piano-core
```

Isso conversa com o servidor real do crates.io mas nunca envia nada — o
final da saída sempre diz `warning: aborting upload due to dry run`, sua
garantia de que nada foi publicado.

## O que esperar em cada passo — resumo

| O que você faz | O que deve acontecer |
|---|---|
| Tocar qualquer nota, como antes | Soa exatamente igual ao M7 — nada mudou no áudio |
| `cargo run --release -p piano-cli -- keyboard` | Continua funcionando exatamente como antes — é o caminho "de sempre" |
| `cargo install piano-cli` | Ainda não funciona — nenhum pacote foi publicado de verdade |
| `gh workflow run release-build.yml` | Compila os três binários e os deixa disponíveis por 14 dias na aba Actions |
| `cargo publish --dry-run -p piano-core` | Passa limpo — este pacote está pronto para publicação real |
| `cargo publish --dry-run -p piano-params` | Falha com "no matching package named piano-core" — esperado, não é um bug, resolve sozinho assim que `piano-core` for publicado de verdade |

Se algum dia você (o dono deste projeto) decidir publicar de verdade ou
criar um Release público, esse é um passo consciente seu — não algo que
qualquer agente automatizado deveria fazer sozinho por você.
