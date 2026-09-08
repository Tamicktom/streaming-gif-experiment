# Objetivo

Verificar se um cliente HTTP, especialmente um navegador exibindo uma tag `<img>`, consegue decodificar e animar um GIF progressivamente enquanto o arquivo ainda está sendo transmitido e não foi encerrado.

Em outras palavras:

- o servidor não entrega um GIF completo de uma vez;
- ele inicia um GIF válido;
- passa a enviar novos frames ao longo do tempo;
- mantém a conexão aberta;
- e observa se o cliente renderiza os frames conforme chegam.

# Hipótese

A hipótese é:

“Como o formato GIF é composto por blocos sequenciais e cada frame pode ser interpretado independentemente após o cabeçalho e a tabela de cores apropriada, alguns decoders serão capazes de exibir uma animação parcial antes do recebimento do trailer final do arquivo.”

Hipótese prática complementar:

“Mesmo que o formato permita decodificação incremental, o comportamento real dependerá do browser, buffering da pilha HTTP e da implementação do decoder de imagem.”

# Pergunta experimental

Pergunta principal:

“Um GIF animado transmitido incrementalmente por uma conexão HTTP mantida aberta pode funcionar como pseudo-stream visual em clientes comuns?”

Subperguntas:
- O browser começa a exibir antes do fim da resposta?
- Ele atualiza a animação à medida que novos frames chegam?
- Existe buffering mínimo perceptível?
- O comportamento varia por navegador?
- A ausência do trailer final impede reprodução contínua ou apenas o encerramento formal?

# Definição operacional do experimento

O experimento consiste em implementar um servidor HTTP em Rust com um endpoint, por exemplo `/live.gif`, que:

1. responde com `Content-Type: image/gif`;
2. envia:
   - header GIF89a,
   - logical screen descriptor,
   - global color table,
   - extensão de loop, se desejado;
3. entra em loop enviando frames completos em intervalos regulares;
4. não envia o trailer `0x3B` até o encerramento do teste, ou nunca envia;
5. mantém a conexão aberta enquanto gera novos frames.

No cliente, o recurso será carregado por algo como:
```html
<img src="/live.gif" />
```

# Variável independente

A principal variável independente é a forma de transmissão incremental do GIF.

Você também pode variar:
- intervalo entre frames, ex. 100 ms, 500 ms, 1 s
- tamanho dos frames
- complexidade visual
- uso de paleta global vs local
- presença ou ausência de extensão de loop
- browser utilizado
- presença de proxy/reverse proxy no meio
- HTTP/1.1 vs HTTP/2, embora HTTP/1.1 seja mais simples para testar

# Variáveis dependentes

O que você mede/observa:
- tempo até o primeiro frame aparecer
- tempo até a animação começar
- continuidade da atualização
- travamentos ou saltos
- latência visual entre geração do frame e exibição
- comportamento quando a conexão fica muito longa
- comportamento ao fechar a conexão sem trailer
- uso de CPU/banda

# Critério de sucesso

O experimento é considerado bem-sucedido se:
- o cliente exibir o primeiro frame sem precisar do arquivo completo;
- e pelo menos alguns frames subsequentes forem renderizados enquanto a conexão continua aberta.

Critério de sucesso forte:
- o cliente se comporta visualmente como um stream rudimentar, atualizando quase em tempo real.

Critério de fracasso:
- o cliente espera o arquivo encerrar;
- ou exibe apenas o primeiro frame;
- ou baixa bytes continuamente mas não atualiza a animação durante a transmissão.

# Arquitetura proposta

## Servidor Rust

Um servidor assíncrono, por exemplo com:
- `tokio`
- `axum`, `hyper` ou `warp`

Responsabilidades:
- abrir endpoint `/live.gif`
- escrever bytes diretamente no corpo da resposta em streaming
- flush periódico
- gerar frames simples dinamicamente

## Gerador de frames

Inicialmente, faça frames sintéticos:
- quadrado andando
- contador visual
- barra mudando
- relógio

Isso evita depender de encoder pesado de vídeo.

## Encoder GIF incremental

Você tem dois caminhos:

### Caminho A: usar crate pronta
Tentar uma crate como `gif` e ver se ela permite emitir frames incrementalmente num writer mantido aberto.

Risco:
- a crate pode pressupor arquivo finito e querer encerrar/trailer no final;
- pode não ser confortável para “stream infinito”.

### Caminho B: escrever o binário manualmente
Como experimento, isso é bem interessante.

Você manualmente escreve:
- header
- logical screen descriptor
- global color table
- application extension de loop
- para cada frame:
  - graphic control extension
  - image descriptor
  - image data

Isso te dá controle total sobre:
- quando flushar
- se mandar trailer
- como organizar as cores
- como deixar a conexão aberta

# Estratégia de simplificação

Para o primeiro experimento, simplifique ao máximo:

- resolução pequena, ex. 64x64
- paleta global fixa com poucas cores
- frames simples
- sem transparência
- sem compressão “inteligente”
- frame completo sempre, sem delta encoding sofisticado

Idealmente:
- fundo preto
- um bloco branco movendo
- 2 a 4 cores totais

Assim o problema vira:
“conseguir emitir um GIF animado sequencial minimamente válido”
em vez de
“fazer um encoder eficiente”.

# Desafios técnicos principais

## 1. LZW
GIF exige dados comprimidos com LZW.

Esse é o ponto mais chato se você for escrever tudo manualmente.

Opções:
- usar crate que já gere o bloco LZW por frame;
- estudar um encoder LZW mínimo;
- trapacear usando frames extremamente simples e uma biblioteca de apoio.

## 2. Buffering
Mesmo se o GIF estiver correto:
- servidor pode bufferizar,
- runtime pode bufferizar,
- proxy pode bufferizar,
- browser pode bufferizar.

Então é importante:
- enviar em chunks,
- fazer flush após cada frame,
- testar sem nginx primeiro,
- usar localhost.

## 3. Comportamento do navegador
O formato permitir não garante que Chrome, Firefox, Safari etc. ajam igual.

# Protocolo experimental sugerido

## Fase 1 — Validação estrutural
Objetivo:
- garantir que o endpoint gera um GIF válido se a conexão for encerrada após N frames.

Procedimento:
- gerar 10 frames,
- enviar trailer,
- abrir no browser,
- verificar animação normal.

## Fase 2 — Streaming prolongado
Objetivo:
- manter a conexão aberta e continuar emitindo frames.

Procedimento:
- gerar 1 frame por segundo,
- não enviar trailer,
- observar se `<img>` atualiza.

## Fase 3 — Comparação entre clientes
Testar:
- Chrome
- Firefox
- Safari, se possível
- talvez `curl` e visualizadores específicos não-browser

## Fase 4 — Sensibilidade a timing
Variar:
- 100 ms/frame
- 500 ms/frame
- 1 s/frame
- 2 s/frame

## Fase 5 — Tolerância ao encerramento
Testar:
- fechar conexão abruptamente sem trailer
- enviar trailer depois de muito tempo
- reconectar

# Resultado esperado

O mais provável é um destes cenários:

1. Melhor caso:
- navegador começa a mostrar cedo
- e vai atualizando conforme chegam novos frames

2. Caso intermediário:
- navegador exibe depois de algum buffer inicial
- depois anima normalmente por blocos

3. Caso ruim:
- navegador só mostra quando a resposta termina
- o experimento falha como pseudo-stream

Meu palpite:
- alguns ambientes vão mostrar progresso parcial,
- mas a confiabilidade vai ser péssima.

# Formulação curta, estilo “descrição de projeto”

Título:
“Transmissão incremental de GIF animado sobre HTTP como pseudo-stream visual”

Resumo:
“Este experimento investiga se um GIF animado pode ser transmitido incrementalmente por uma conexão HTTP persistente e ser renderizado progressivamente por navegadores antes do encerramento formal do arquivo. Um servidor em Rust emitirá o cabeçalho do GIF e, em seguida, enviará frames completos em intervalos regulares, sem necessariamente transmitir o trailer final. O objetivo é avaliar a viabilidade estrutural e prática do uso de GIF como mecanismo improvisado de streaming visual.”

# Formulação mais objetiva

Problema:
- GIF é formato de arquivo, não protocolo de streaming.
- Mas seus dados são sequenciais e potencialmente decodificáveis de forma incremental.

Tese experimental:
- Um servidor Rust pode explorar essa característica para enviar um GIF “aberto” por tempo indeterminado.

Método:
- Servir um endpoint HTTP com corpo chunked e conexão persistente.
- Emitir header + paleta + frames de animação ao longo do tempo.
- Medir o comportamento de renderização em navegadores.

# Stack Rust sugerida

Se você quiser algo pragmático:
- `tokio`
- `axum`
- `bytes`
- `tokio-stream`
- possivelmente `gif`

Se quiser controle total:
- `tokio`
- `hyper`
- escrita manual dos bytes do GIF

# Minha recomendação

Para esse experimento específico:
- use Rust para o servidor,
- mas não tente começar com “encoder GIF completo do zero”;
- comece com uma biblioteca para gerar frames válidos,
- e só depois, se ela atrapalhar, passe para emissão binária manual.