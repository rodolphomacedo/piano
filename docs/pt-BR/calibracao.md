# Calibração: ajustar o piano medindo um piano de referência

Este guia explica, sem jargão, o plano para parar de ajustar o som só de
ouvido. É o marco **M19** no GitHub, issues
[#123](https://github.com/rodolphomacedo/piano/issues/123) a
[#127](https://github.com/rodolphomacedo/piano/issues/127). O desenho
técnico completo, em inglês, está em
[`docs/CALIBRATION.md`](../CALIBRATION.md).

**Ainda não há nada para rodar.** Este documento descreve o que vai ser
construído e por quê.

## O problema

O modelo tem uns 40 controles que mudam o som: perda da corda, dureza do
martelo, tampo, acoplamento entre cordas e outros. Até agora a gente
comparava o som com a **teoria**. Por exemplo, "num piano de verdade, a
oitava parcial morre bem antes da fundamental". Isso achou muitos defeitos.

As reclamações de agora, porém, são outra coisa: *"parece um violão"*
(#122) e *"a batida do martelo está alta demais"* (#121). Essas frases
comparam o som com um **piano de verdade**, e a teoria não diz o quanto a
batida de um Lá grave deveria estar abaixo do som da nota. Para isso é
preciso um **alvo medido**.

Ajustar 40 controles de ouvido também não converge: mexer num muda o
efeito dos outros.

## A ideia em quatro passos

1. **Tocar exatamente as mesmas notas em três instrumentos** (#124).
   Um arquivo MIDI fixo, com as mesmas notas e as mesmas forças de toque, é
   tocado no nosso modelo, no piano do GarageBand e no seu Yamaha P-125.
   Ninguém toca à mão, senão a gente estaria medindo a mão e não o piano.

2. **Medir as gravações com a mesma régua** (#125).
   O `piano analyze`, que você já usa, passa a aceitar um arquivo WAV de
   fora e a mostrar o modelo e a referência lado a lado:
   - quanto tempo cada parcial leva para cair;
   - o quanto as parciais se afastam das múltiplas exatas (inarmonicidade);
   - a força do ataque em relação ao som que vem depois, que é o número
     da #121;
   - como o brilho muda ao longo da nota.

3. **Descobrir quais controles as medidas enxergam** (#126).
   Você disse que ainda não sabe quais parâmetros são necessários, e
   ninguém sabe ainda. Por isso esta etapa existe: mexemos em cada controle,
   um de cada vez, e medimos o que muda. O resultado é uma tabela "controle ×
   medida". Ela mostra:
   - quais controles dá para estimar a partir de uma gravação;
   - quais não aparecem em gravação nenhuma, e por isso não adianta tentar;
   - quais pares fazem a mesma coisa e não dá para separar. Por exemplo, o
     volume geral e a curva de velocidade.

4. **Estimar com inferência bayesiana** (#127), num repositório separado.
   Em vez de um número só ("a dureza do martelo é 4,2"), o resultado é uma
   distribuição ("está entre 3,8 e 4,6, provavelmente perto de 4,2"). Se a
   faixa sair tão larga quanto o chute inicial, a medida não enxerga aquele
   controle, e isso também é uma resposta útil.

   A ordem é pensada para que cada etapa possa falhar barato:
   - **Etapa 0, sem gravação nenhuma:** o próprio motor gera notas com
     controles *conhecidos*. Verificamos se a estimação consegue
     recuperá-los. Se não consegue recuperar os próprios valores, não vai
     recuperar os de um Yamaha.
   - **Etapa 1:** os controles que têm fórmula física direta. A
     inarmonicidade sai das frequências das parciais, e as perdas da corda
     saem dos tempos de queda. O modelo é hierárquico entre as teclas, para
     gerar curvas suaves ao longo do teclado.
   - **Etapa 2:** os controles sem fórmula, como martelo, ponto de batida e
     tampo, usando o próprio simulador.
   - **Validação:** estimamos com algumas teclas e conferimos se o
     resultado prevê as teclas que ficaram de fora.
   - **Resultado final:** um arquivo `.piano.json` que você abre no estúdio
     e **escuta**. O ouvido continua sendo o juiz final, mas agora julga um
     candidato que as medidas já aproximaram.

## As regras que continuam valendo

- **Nenhuma gravação entra neste repositório.** O som do P-125 e do
  GarageBand também vem de gravações de piano de verdade, feitas por
  outras pessoas. Eles servem só de régua: o áudio fica no seu computador
  ou no repositório de calibração, fora do Git. Aqui entram apenas números,
  como tempos de queda e valores estimados. Essa decisão vai ser escrita
  num documento próprio (#123) antes de qualquer código ler um WAV.
- Igualar o P-125 não é o objetivo final. É o primeiro teste do **método**.
  Quando ele funcionar, dá para apontar para outro piano de referência.

## O que você vai precisar fazer (quando chegar a hora)

Quando a #124 for feita, vai sair um guia passo a passo,
`docs/pt-BR/calibracao-como-gravar.md`, para:

1. gerar o arquivo MIDI de teste com um comando;
2. abrir esse arquivo no GarageBand e exportar em WAV;
3. mandar o mesmo arquivo do computador para o P-125 e gravar o som que
   sai dele. Ainda falta confirmar se o P-125 manda áudio pelo cabo USB ou
   só MIDI. Se for só MIDI, a gravação sai pela saída de fone, com uma
   interface de áudio;
4. anotar as configurações usadas, incluindo a sensibilidade ao toque do
   P-125, porque ela muda a força de cada nota.
