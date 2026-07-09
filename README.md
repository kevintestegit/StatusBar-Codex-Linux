# StatusBar Codex Linux

Um applet nativo de bandeja do sistema Linux para monitorar uso do Codex sem abrir o terminal.

![Dashboard](dashboard.png)

## Como funciona

O app lê arquivos JSONL de sessão do Codex em disco, analisa eventos `token_count` e `rate_limits`, e exibe na bandeja do sistema:

- **Limites de taxa** — uso da janela de 5h e semanal, com contagem regressiva para reset
- **Custo estimado** — equivalente API pública por dia, mês e total histórico
- **Consumo por modelo** — breakdown de tokens input/cached/output/reasoning por modelo (GPT-5, GPT-5.4, etc.)
- **Ritmo de uso** — se está dentro do esperado ou queimando o limite muito rápido

### Arquitetura

```
~/.codex/sessions/*.jsonl
         │
         ▼
  ┌─────────────────┐
  │  Parse JSONL    │  ───  lê eventos token_count e rate_limits
  │  (walkdir)      │
  └──────┬──────────┘
         │
         ▼
  ┌─────────────────┐
  │  Aggregate      │  ───  agrupa por modelo, dia, mês
  │  (HashMap)      │
  └──────┬──────────┘
         │
         ▼
  ┌─────────────────┐
  │  GTK3 Tray      │  ───  AppIndicator com menu, labels, submenus
  │  + Cairo        │       confete fullscreen (party mode)
  └─────────────────┘
         │
         ▼
    Bandeja do sistema
    "5h 1% | $0.00"
```

### Fontes de dados

1. **App server live** (primário) — usa `codex app-server --listen stdio://` via JSON-RPC para buscar `account/rateLimits/read` em tempo real
2. **JSONL local** (fallback) — lê `$CODEX_HOME/sessions/` ou `~/.codex/sessions/` quando o app server não responde
3. **Cache de arquivos** — cada arquivo JSONL é cacheado por tamanho + timestamp; só reparsa se mudou

### Modos de execução

| Flag | Descrição |
|------|-----------|
| _(sem flag)_ | Abre o tray icon e fica em background |
| `--once` | Imprime resumo no terminal e sai |
| `--html` | Gera dashboard HTML completo no stdout |
| `--test-5h-reset` | Simula notificação de reset 5h |
| `--test-weekly-reset` | Simula notificação de reset semanal + confete |
| `--test-pace-alert` | Simula alerta de ritmo acelerado |

### Funcionalidades

- Indicador na bandeja com cor dinâmica (verde → amarelo → vermelho conforme uso)
- Menu com rate limits, custos, tokens, modo festa, intervalo de refresh
- Notificações desktop em reset de janela e ritmo acelerado
- Modo festa: overlay fullscreen com confete Cairo quando o limite semanal reseta
- Dashboard HTML detalhado com tabela por modelo
- Ícones SVG gerados dinamicamente no `/tmp` com percentual
- Cache inteligente: não reparsa arquivos JSONL inalterados

### Configuração

Arquivo: `~/.config/codex-usage-tray/config.json`

```json
{
  "party_mode": false,
  "refresh_seconds": 30
}
```

Ajustável via menu da bandeja (5s, 15s, 30s, 1min, 5min).

### Build

```bash
# Dependências (Debian/Ubuntu)
sudo apt install cargo pkg-config libgtk-3-dev libayatana-appindicator3-dev libgtk-layer-shell-dev

cargo build --release
./target/release/codex-usage-tray
```

### Stack

- **Rust** — binário nativo, sem runtime web
- **GTK3** — menu, labels, janelas
- **Ayatana AppIndicator** — integração com bandeja Linux
- **GTK Layer Shell** — overlay Wayland para modo festa
- **Cairo** — desenho de confete
- **JSONL parsing** — leitura local de sessões Codex

### Privacidade

O app **não envia dados para lugar nenhum**. Lê apenas arquivos locais em `$CODEX_HOME/sessions` ou `~/.codex/sessions`. Sem telemetria, sem rede, sem banco de dados.
