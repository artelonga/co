---
created: 2026-05-17T00:00:00+00:00
modified: 2026-05-17T00:00:00+00:00
order: 23
slug: conta-e-mensagens
tags:
- conta
- login
- mensagens
- chat
- universos
title: Conta e mensagens
type: page
---

# Conta e mensagens

Como funciona entrar, mudar de senha, conversar e ser convidado para um universo. Linguagem direta, sem jargão técnico.

## Sua conta

Sua conta é uma única identidade no Co. Ela funciona em **todos os universos** que você participa — não importa se é o seu universo pessoal, um universo público como o [Quilombo Araucária](https://quilomboaraucaria.com.br), ou um universo privado para o qual alguém te convidou.

### Como entrar

Quatro caminhos. Todos te deixam na mesma conta.

#### 1. Email com código de verificação (recomendado para a primeira vez)

1. Clique em **Entrar** no canto superior direito
2. Digite seu email
3. Receba um código de 6 dígitos no email (válido por 5 minutos)
4. Digite o código e pronto

Esse é o jeito mais simples — você não precisa criar uma senha de cara. Se quiser, define uma senha depois nas configurações.

> Limite: 3 códigos por email a cada 15 minutos, para evitar abuso.

#### 2. Senha

Se você já tem uma senha cadastrada:

1. **Entrar** → clique em **"Já tem usuário e senha? ▼"**
2. Email + senha → entrar

Senhas são guardadas em hash (Argon2id) — ninguém da plataforma vê sua senha original, nem mesmo o administrador. [Mais detalhes na página de Segurança](/seguranca).

#### 3. Google

Funciona com qualquer conta Google. Um clique e pronto. Útil se você não quer guardar mais uma senha.

#### 4. Convite direto

Quando alguém te convida para o universo deles, você recebe um link especial. Se você ainda não tem conta, o link te cria uma. Se já tem, te adiciona automaticamente ao universo dessa pessoa.

### Sua sessão

Depois que você entra, fica logado por **7 dias** sem precisar fazer login de novo (no mesmo navegador). Se você sair ou limpar cookies, precisa entrar de novo.

A "sessão" é só um cookie no seu navegador chamado `session`. Quando ele expira ou some, você precisa autenticar de novo.

## Universos

Um universo é seu espaço — onde você guarda conteúdo, organiza tarefas, conversa com pessoas.

Três tipos:

| Tipo | Quem vê | Quem edita |
|---|---|---|
| **Público** | qualquer pessoa, mesmo sem login | só o dono e membros |
| **Privado** | só o dono e membros | só o dono e membros |
| **Template** | qualquer pessoa, é o ponto de entrada para visitantes anônimos | ninguém edita o template em si — você cria uma cópia para si |

### Convidar alguém para o seu universo

Você pode adicionar pessoas ao seu universo como **membros**. Membros podem:

- Ler tudo (mesmo conteúdo privado)
- Criar e editar conteúdo diretamente
- Conversar nos canais de chat do universo
- Ver as estatísticas (storage, número de entradas, etc.)

Para convidar:

1. Abra o universo
2. Clique no botão **ℹ** no cabeçalho
3. Procure a seção "Membros" → "Convidar"
4. Digite o email da pessoa ou compartilhe o link de convite gerado

A pessoa recebe um link único que serve UMA vez. Aceitando o link, ela vira membro do seu universo.

### Sugerir mudança sem ser membro

Se você está num universo público mas **não** é membro (ou é membro só pra ler), você pode propor mudanças sem ter permissão de escrita:

1. Clique numa página, clique pra editar
2. Faça sua alteração
3. Clique em **Salvar**
4. Se você não tem permissão de escrita, o sistema pergunta: *"Enviar essa mudança como proposta para revisão?"*
5. Aceitando, sua proposta vai para a caixa de entrada do dono do universo

O dono vê suas propostas, pode aceitar (sua mudança vira oficial) ou rejeitar. Você é notificado da decisão.

## Mensagens

Três jeitos de conversar dentro do Co:

### 1. Canais de chat por universo

Cada universo tem canais de conversa que membros usam para discutir o trabalho. É como o Slack ou Discord, mas dentro do universo.

- Você abre a barra lateral **Conversas** (ícone de balão no cabeçalho)
- Vê os canais do universo atual e suas mensagens diretas
- Mensagens chegam em tempo real (sem precisar recarregar a página)
- Pode @mencionar alguém — eles recebem notificação

### 2. Mensagens diretas (DMs)

Conversa privada entre duas pessoas, independente de universo. Para iniciar:

1. Abra **Conversas**
2. Clique numa pessoa que você conhece (na lista de membros de algum universo compartilhado) ou busque por nome
3. Mande mensagem direta

**Política de DMs**: você pode controlar quem pode te mandar DM:
- **Qualquer um**: qualquer pessoa autenticada
- **Só seguidores**: só pessoas que já têm alguma relação com você no platform
- **Desligado**: ninguém pode te mandar DM

Configurável em **Configurações** → **Privacidade**.

### 3. Propostas como mensagens

Quando alguém propõe uma mudança no seu universo (veja "Sugerir mudança" acima), você recebe **uma notificação** com:
- Quem propôs
- Em qual página
- Link para revisar

Você pode aceitar ou rejeitar diretamente da notificação.

## Notificações

Tudo que precisa da sua atenção aparece em três canais:

1. **🔔 Sininho** — no cabeçalho do app. Você vê tudo em tempo real, conta de não-lidas no badge.
2. **Email** — para coisas mais importantes. Você decide o que recebe em **Configurações** → **Notificações**.
3. **Push** *(se autorizou no navegador)* — funciona em celular e desktop, mesmo com o navegador fechado. Útil para conversas e propostas.

### O que gera notificação?

- **Mensagem no chat** com sua menção `@você`
- **Mensagem direta** nova
- **Convite** para outro universo
- **Proposta de mudança** chegou no seu universo (se você é o dono)
- **Sua proposta** foi aceita ou rejeitada
- **Comentário** numa entrada que você criou

## Atravessando universos (cross-domain)

Você está logado no Co. Clica num link e vai para o [Quilombo Araucária](https://quilomboaraucaria.com.br). **Você continua logado lá**, como a mesma conta.

Como isso funciona, em linguagem simples:

1. Quando você visita o Quilombo pela primeira vez, ele te redireciona rapidinho para o Co
2. O Co identifica você (cookie de sessão) e devolve um "passaporte" (token) curto para o Quilombo
3. O Quilombo usa esse passaporte para emitir seu próprio cookie pra você
4. Depois disso, sua sessão no Quilombo dura **7 dias**, igual no Co

Você só "passa" pelo Co uma vez. Da próxima vez que abrir o Quilombo, já entra direto.

> Esse fluxo é **anônimo** do ponto de vista do tráfego entre os sites — eles só compartilham que você é o usuário X, nada mais. Não tem cookie compartilhado, não tem rastreamento entre domínios.

Detalhes técnicos para quem quiser: [Segurança](/seguranca).

## Limites e privacidade

- **Anônimos** (sem login) podem **ler** universos públicos e a parte `public/` dos universos com convenção de visibilidade pública. Para escrever, criar conteúdo ou conversar, você precisa estar logado.
- **Sem rastreamento entre serviços**: o Co não compartilha sua identidade com Google Analytics, Facebook Pixel ou similares. [Detalhes em Dados Rastreados](/dados-rastreados).
- **Sua conta é portável**: você pode exportar seu universo (Markdown puro) a qualquer momento via API ou CLI. Detalhes em [Visualizadores de Markdown](/renderers).

## Resolvendo problemas

### "Esqueci minha senha"

1. Clique em **Entrar** → digite seu email
2. Em vez de senha, peça **código por email** (sempre funciona)
3. Depois de entrar, vá em **Configurações** → **Senha** → defina uma nova

### "Não recebi o código"

- Cheque spam / lixeira (origem: `senhas@artelonga.com.br`)
- Se passou 5 minutos, peça outro
- Se já pediu 3 vezes em 15 minutos, espere até esgotar a janela

### "Saí sem querer"

Cookies foram limpos. Entre de novo — sua conta + universos estão intactos.

### "Quero apagar minha conta"

Mande email para `yuri@artelonga.com.br` pedindo. Suporte LGPD: sua conta e todo conteúdo associado serão removidos em até 30 dias. Backups de DR são purgados em 90 dias (retenção do bucket S3). Mais em [Privacidade](/privacidade).

---

**Veja também**: [Segurança](/seguranca) · [Licença](/licensa) · [Dados Rastreados](/dados-rastreados) · [Visualizadores](/renderers)
