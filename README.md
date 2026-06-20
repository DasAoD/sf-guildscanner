# ⚔️ SF Gilden-Scanner

> **搌 Mirror-Hinweis:** Dieses Repository ist ein automatischer Spiegel.
> Die primäre Entwicklung findet auf **[git.uliana.de/DasAoD/sf-guildscanner](https://git.uliana.de/DasAoD/sf-guildscanner)** statt.
> Issues und Pull Requests bitte dort öffnen.

![Docker Hub](https://img.shields.io/docker/v/dasaod/sf-guildscanner?label=Docker%20Hub&logo=docker)(https://hub.docker.com/r/dasaod/sf-guildscanner)
[![GitHub](https://img.shields.io/badge/GitHub-DasAoD%2Fsf--guildscanner-blue?logo=github)](https://github.com/DasAoD/sf-guildscanner)

Findet angreifbare Gilden in Shakes & Fidget – basierend auf Mitgliederanzahl und den einzelnen Leveln der Mitglieder.

## Funktionsweise

### Scan-Ablauf (vollautomatisch)

1. **Login** per SSO → Charakter auswählen → Gildendaten laden
2. **Phase 1 – HoF-Scan**: Scannt die komplette Gilden-Ehrenhalle und ermittelt alle Gilden mit Rang, Ehre, Mitgliederzahl
3. **Automatische Filterung**: Markiert alle angreifbaren Gilden nach der Spielregel:
   - Alle Gilden **unter** euch (höherer Rang) > angreifbar
   - Gilden **über** euch → angreifbar wenn **max. 20 Ränge** ODER **max. 3000 Ehru** darüber
4. **Phase 2 ↓ Detail-Scan**: Für jede angreifbare Gilde wird ViewGuild aufgerufen → alle Mitglieder mit ihren individuellen Leveln
5. **Speicherung**: Ergebnisse werden als JSON-Datei persistiert

### Filter-Möglichkeiten

- **Max Mitglieder**: Nur Gilden mit höchstens X Mitgliedern anzeigen
- **Max höchstes Level**: Nur Gilden, deren stärkster Charakter höchstens Level X hat
- **Angegriffene ausblenden**: Gilden die gerade angegriffen werden verstecken

## Deployment

### Docker Hub (empfohlen – kein Bauen nötig)

```bash
mkdir -p /mnt/user/appdata/sfguild-scanner
cd /mnt/user/appdata/sfguild-scanner
curl -O https://raw.githubusercontent.com/DasAoD/sf-guildscanner/main/docker-compose.yml
docker compose up -d
```

### Manuell ohne docker-compose

```bash
docker run -d \
  --name sfguild-scanner \
  -p 8085:8080 \
  -v ./data:/app/data \
  -e RUST_LOG=info \
  --restart unless-stopped \
  dasaod/sf-guildscanner:latest
```

## Mitwirkende

Dieses Projekt wurde in Zusammenarbeit mit [Claude](https://claude.ai) (Sonnet 4.6) von [Anthropic](https://anthropic.com) entwickelt und iterativ ausgebaut.  
Der überwiegende Teil des Codes, der Architektur und der Dokumentation wurde durch KI generiert und gemeinsam verfeinert.

| Rolle | Person / Tool |
|---|---|
| Projektidee, Anforderungen & Tests | [DasAoD](https://git.uliana.de/DasAoD) |
| Code, Architektur, Dokumentation | [Claude](https://git.uliana.de/Claude) (Anthropic) |

## License

[MIT](LICENSE)
