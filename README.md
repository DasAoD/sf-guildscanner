# ⚔️ SF Gilden-Scanner

[![Docker Hub](https://img.shields.io/docker/v/dasaod/sf-guildscanner?label=Docker%20Hub&logo=docker)](https://hub.docker.com/r/dasaod/sf-guildscanner)
[![GitHub](https://img.shields.io/badge/GitHub-DasAoD%2Fsf--guildscanner-blue?logo=github)](https://github.com/DasAoD/sf-guildscanner)

Findet angreifbare Gilden in Shakes & Fidget – basierend auf Mitgliederanzahl und den einzelnen Leveln der Mitglieder.

## Funktionsweise

### Scan-Ablauf (vollautomatisch)

1. **Login** per SSO → Charakter auswählen → Gildendaten laden
2. **Phase 1 – HoF-Scan**: Scannt die komplette Gilden-Ehrenhalle und ermittelt alle Gilden mit Rang, Ehre, Mitgliederzahl
3. **Automatische Filterung**: Markiert alle angreifbaren Gilden nach der Spielregel:
   - Alle Gilden **unter** euch (höherer Rang) → angreifbar
   - Gilden **über** euch → angreifbar wenn **max. 20 Ränge** ODER **max. 3000 Ehre** darüber
4. **Phase 2 – Detail-Scan**: Für jede angreifbare Gilde wird `ViewGuild` aufgerufen → alle Mitglieder mit ihren individuellen Leveln
5. **Speicherung**: Ergebnisse werden als JSON-Datei persistiert (beim nächsten Start wieder verfügbar)

### Filter-Möglichkeiten

- **Max Mitglieder**: Nur Gilden mit höchstens X Mitgliedern anzeigen
- **Max höchstes Level**: Nur Gilden, deren stärkster Charakter höchstens Level X hat
- **Angegriffene ausblenden**: Gilden die gerade angegriffen werden verstecken

### Vergleich

Für jede Gilde siehst du:
- Mitgliederanzahl vs. deine Gilde
- Max/Min Level und Gesamt-Level-Summe
- Im Detail-View: Alle Mitglieder mit Level + direkter Vergleich

### Export

Der komplette Scan lässt sich als JSON exportieren (Button "📥 Export"), inklusive aller Mitglieder-Level.

## Deployment

### Docker Hub (empfohlen – kein Bauen nötig)

```bash
mkdir -p /mnt/user/appdata/sfguild-scanner
cd /mnt/user/appdata/sfguild-scanner

# docker-compose.yml herunterladen
curl -O https://raw.githubusercontent.com/DasAoD/sf-guildscanner/main/docker-compose.yml

# Starten (Image wird automatisch von Docker Hub geladen)
docker compose up -d

# Öffnen: http://<HOST-IP>:8085
```

Die Scan-Daten werden persistent im `./data/` Ordner gespeichert.

### Aus dem Quellcode bauen

```bash
git clone https://github.com/DasAoD/sf-guildscanner.git
cd sf-guildscanner
docker compose -f docker-compose.build.yml up -d --build
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

## Scan-Dauer

- **Phase 1** (HoF): ~10-40 Sekunden (abhängig von der Servergröße)
- **Phase 2** (Details): ~0,5 Sek. pro Gilde → bei 100 angreifbaren Gilden ca. 1 Minute

Der Fortschritt wird live im Browser angezeigt.

## Projektstruktur

```
sfguild-scanner/
├── Cargo.toml
├── Dockerfile              # Multi-Stage Build
├── docker-compose.yml
├── .dockerignore
├── src/
│   └── main.rs             # Rust Backend (Axum + sf-api)
├── static/
│   └── index.html          # Web-Frontend
└── data/                   # Persistente Scan-Daten (Volume)
    └── scan_*.json
```

## Hinweise

- Login-Daten werden **nur im RAM** gehalten, nie gespeichert
- Der Container braucht Zugang zu `sfgame.net` und `sso.playa-games.com`
- Erster Build dauert 3-5 Min. (Rust kompiliert alle Dependencies)
- Zwischen API-Requests wird 400-500ms Pause gehalten (Rate-Limiting)

## Credits

- [sf-api](https://github.com/the-marenga/sf-api) by [the-marenga](https://github.com/the-marenga) — Rust library for the Shakes & Fidget API

---

## Mitwirkende

Dieses Projekt wurde in Zusammenarbeit mit [Claude](https://claude.ai) (Sonnet 4.6) von [Anthropic](https://anthropic.com) entwickelt und iterativ ausgebaut.  
Der überwiegende Teil des Codes, der Architektur und der Dokumentation wurde durch KI generiert und gemeinsam verfeinert.

| Rolle | Person / Tool |
|---|---|
| Projektidee, Anforderungen & Tests | [DasAoD](https://git.uliana.de/DasAoD) |
| Code, Architektur, Dokumentation | [Claude](https://git.uliana.de/Claude) (Anthropic) |

## License

[MIT](LICENSE)
