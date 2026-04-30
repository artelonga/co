---
created: 2026-04-11T01:26:20.515990+00:00
modified: 2026-04-26T00:00:00+00:00
order: 10
slug: termos
tags:
- legal
title: Termos de Uso
type: page
---

# Termos de Uso

Última atualização: abril de 2026

## 1. Aceitação

Ao usar o Co em `co.artelonga.com.br` (ou qualquer instância derivada), você concorda com estes Termos. Se não concordar, não use a Plataforma.

## 2. O que é o Co

O Co é uma plataforma open-source (licença MIT) de **gestão de conteúdo em grafo**. Você cria, organiza e compartilha conteúdo em Markdown através de **universos** — espaços de trabalho pessoais ou colaborativos.

Você pode usar o Co de duas formas:

| Modelo | URL exemplo | Quem hospeda | Quem é controlador |
|--------|-------------|--------------|--------------------|
| **Auto-hospedado** | seu próprio servidor | Você | Você |
| **Gerenciado pela Arte Longa** | `co.artelonga.com.br` | Arte Longa via Fly.io | Arte Longa |

Estes Termos cobrem o modelo gerenciado. Para auto-hospedagem, a licença MIT é o único contrato.

## 3. Estado do produto: teste público inicial

**Atenção:** o Co está em **teste público inicial** (v1.x). Espere:
- Bugs ocasionais
- Mudanças de comportamento entre versões
- Ausência de SLA formal
- Funcionalidades roadmap ainda não implementadas (cifragem em repouso, sync mobile, app desktop nativo)

Para uso em produção crítica, **aguarde a v3.0** ou auto-hospede com revisão sua.

## 4. Contas e universos

- **Visitantes (sem conta):** podem criar e editar até **100 entradas** em um universo anônimo local.
- **Conta:** você é responsável pela segurança das suas credenciais. Senhas seguem hash Argon2id.
- **Universos privados:** visíveis apenas ao proprietário e membros explicitamente convidados.
- **Universos públicos:** acessíveis a qualquer pessoa com o link; ainda assim, edição requer permissão.
- **Limites técnicos:** podem ser aplicados para evitar abuso da infraestrutura compartilhada.

## 5. Conteúdo do usuário

- **Você mantém todos os direitos** sobre o conteúdo que cria.
- **Você concede à Arte Longa** uma licença não-exclusiva e gratuita estritamente para armazenar, exibir e processar seu conteúdo no escopo do serviço (zero direito de uso para outros fins).
- O conteúdo é armazenado como Markdown e **exportável a qualquer momento** via Vault API ou download direto.
- Você é responsável pelo conteúdo que cria. Conteúdo ilegal poderá ser removido sem aviso prévio.

## 6. Uso aceitável

Você concorda em **não**:
- Distribuir conteúdo ilegal, difamatório ou que viole direitos de terceiros (incluindo direitos autorais)
- Tentar acessar contas, universos ou dados de outros usuários sem autorização
- Sobrecarregar intencionalmente a infraestrutura (DoS, scraping abusivo, requisições automatizadas em volume desproporcional)
- Usar a Plataforma para enviar spam, malware ou phishing
- Realizar engenharia reversa com intuito malicioso (revisão de segurança ética é bem-vinda — escreva para yuri@artelonga.com.br)

## 7. Disponibilidade

A Plataforma é oferecida **"como está" e "conforme disponível"**, sem garantias de:
- Disponibilidade contínua (esperamos >99% mas não há SLA contratual)
- Ausência de bugs
- Adequação a propósito específico

Faremos esforços razoáveis para manter o serviço operacional. Manutenções programadas serão anunciadas com antecedência quando possível.

## 8. Privacidade e segurança

- O tratamento de dados pessoais segue a [Política de Privacidade](/co/template?path=content/privacidade.md).
- Conteúdo é protegido por TLS em trânsito e controle de acesso por universo.
- **Cifragem dos corpos em repouso ainda não está implementada** (roadmap v3.0 — CO-86). Operadores da instância gerenciada conseguem tecnicamente acessar conteúdo no servidor. Para conteúdo sensível, use auto-hospedagem.

## 9. Encerramento de conta

- **Por você:** Configurações → Excluir conta. Dados removidos em até 30 dias.
- **Por nós:** podemos encerrar contas que violem estes Termos, com aviso prévio quando viável. Você pode exportar seus dados antes do encerramento.

## 10. Limitação de responsabilidade

Na máxima extensão permitida pela legislação brasileira, a Arte Longa não se responsabiliza por:
- Perda de dados decorrente de força maior, falha de infraestrutura de terceiros (Fly.io), ou ataques externos
- Lucros cessantes ou danos indiretos
- Conteúdo gerado por outros usuários

A responsabilidade total da Arte Longa, em qualquer hipótese, fica limitada ao valor pago pelo usuário nos últimos 12 meses (atualmente: zero, pois o serviço é gratuito durante o teste público).

## 11. Modificações

Estes Termos podem ser atualizados. Mudanças significativas serão comunicadas via banner na Plataforma com **pelo menos 15 dias de antecedência**. Histórico em <https://github.com/artelonga/co/commits/main/data/universes/template/content/termos.md>.

## 12. Lei aplicável e foro

Estes Termos são regidos pela legislação brasileira. Foro eleito: Curitiba/PR, exceto onde a lei conferir foro privilegiado ao consumidor.

## 13. Contato

Dúvidas, denúncias ou solicitações: yuri@artelonga.com.br

---

*Arte Longa — Curitiba, PR, Brasil*
