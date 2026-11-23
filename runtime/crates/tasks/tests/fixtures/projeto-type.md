---
kind: mdbase.type
name: projeto
version: 1
description: Projeto (mínimo; campos completos quando houver uso real).
schema:
  dialect: json-schema-2020-12
  value:
    $schema: https://json-schema.org/draft/2020-12/schema
    type: object
    additionalProperties: true
    allOf:
      - if:
          properties:
            status:
              enum: [pausa, aprovacao, concluido, cancelado]
          required: [status]
        then:
          required: [outcome]
      - if:
          properties:
            status:
              const: pausa
          required: [status]
        then:
          required: [resumeOn]
    properties:
      type:
        const: projeto
      name:
        type: string
        minLength: 1
      brand: &id001
        type: string
        pattern: ^\[\[.+\]\]$
      owner: *id001
      status:
        enum: [backlog, to-do, em-andamento, aprovacao, pausa, concluido, cancelado]
      outcome:
        type: string
        minLength: 1
      outcomeDate:
        type: string
        format: date
      resumeOn:
        type: string
        format: date
      nextMilestone:
        type: string
        minLength: 1
      nextMilestoneDate:
        type: string
        format: date
      priority:
        enum: [nenhuma, baixa, normal, alta]
---

# Projeto

Empreitada com objetivo, começo e fim, pertencente a uma marca. Mora em `02-marcas/{marca}/Projetos/`. Campos: `brand` (link), `owner` (uma pessoa de `11-pessoas/`), `status` (os mesmos da tarefa: `backlog`, `to-do`, `em-andamento`, `aprovacao`, `pausa`, `concluido`, `cancelado`), `outcome` (o desfecho), `outcomeDate`, `resumeOn` (só em `pausa`), `nextMilestone` e `nextMilestoneDate` (próximo marco) e o objetivo em uma frase. `pausa`, `aprovacao`, `concluido` e `cancelado` exigem `outcome`, e `pausa` exige `resumeOn`: o `mdbase validate` acusa o projeto arrastado no kanban sem desfecho. **`status`, `owner`, `outcome`, `outcomeDate` e `resumeOn` só pelo `tasknotes-agent`** (`project-status`, `project-assign`), nunca à mão: ver [[ciclo-de-vida-de-projetos-design]]. Molde em `08-templates/projeto.md`. **Nunca recebe a tag `task`**. Notas de outros tipos apontam para o projeto pelo campo `project`.

Convenções comuns a toda nota da base (chave do tipo, tags, marca, projeto e pessoa por wikilink entre aspas, aresta no grafo pelo campo `related:`) e o vocabulário de `tipo`: `_config/mdbase/convencoes.md`.
