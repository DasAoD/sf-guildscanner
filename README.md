[![Docker Hub](https://img.shields.io/docker/v/dasaod/sf-guildscanner?label=Docker%20Hub&logo=docker)](https://hub.docker.com/r/dasaod/sf-guildscanner)
[![GitHub](https://img.shields.io/badge/GitHub-DasAoD%2Fsf--guildscanner-blue?logo=github)](https://github.com/DasAoD/sf-guildscanner)
# ⚔️ SF Gilden-Scanner

> **📌 Mirror-Hinweis:** Dieses Repository ist ein automatischer Spiegel.
> Die primäre Entwicklung findet auf **[git.uliana.de/DasAoD/sf-guildscanner](https://git.uliana.de/DasAoD/sf-guildscanner)** statt.
> Issues und Pull Requests bitte dort öffnen.


Findet angreifbare Gilden in Shakes & Fidget – basierend auf Mitgliederanzahl und den einzelnen Leveln der Mitglieder.

## Funktionsweise

### Scan-Ablauf (vollautomatisch)

1. **Anmelden** am Scanner, dann **S&F-Login** per SSO → Charakter auswählen → Gildendaten laden
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

### Benutzer und Anmeldung

Der Scanner hat eigene Benutzer. Beim ersten Aufruf legst du den ersten Benutzer an, er wird Admin und legt unter *Benutzer* weitere an. Jeder sieht nur seine eigenen Scans (`data/users/<id>/`).

**Die Einrichtung ist für den ersten Aufrufer offen.** Richte den Scanner deshalb direkt nach dem ersten Start ein, bevor der Port für andere erreichbar ist. Ist `data/users.json` beschädigt oder nicht lesbar, startet der Scanner absichtlich nicht, statt die Einrichtung erneut zu öffnen.

- Passwörter werden mit Argon2id gespeichert (`data/users.json`), Sitzungen nur als Hash (`data/sessions.json`), Mindestlänge 10 Zeichen.
- Die Anmeldung bleibt 30 Tage bestehen und verlängert sich bei Nutzung. Nach mehreren Fehlversuchen wird der Login zeitweise gebremst.
- Wer einen Benutzernamen kennt, kann dieses Konto durch gezieltes Falsch-Anmelden für höchstens 15 Minuten ausbremsen (kein hartes Sperren, damit niemand den Admin aussperren kann).
- `SFGS_TRUST_PROXY=true` lässt den Scanner die Client-Adresse aus dem letzten `X-Forwarded-For`-Eintrag lesen und zusätzlich pro Adresse bremsen. Nur setzen, wenn der Port ausschließlich über deinen Proxy erreichbar ist, sonst kann ein Client den Header fälschen.
- Das Sitzungs-Cookie ist `Secure`: Der Scanner ist für den Betrieb **hinter einem HTTPS-Reverse-Proxy** gedacht. Den Port nicht zusätzlich ohne Proxy ins Netz öffnen. Für lokale Tests über http geht `SFGS_COOKIE_SECURE=false`.
- Scans aus Versionen vor 1.0.0 (direkt in `data/`) werden nicht mehr gelesen und können gelöscht werden.

### S&F-Konten speichern

Unter *Konten* hinterlegst du pro Benutzer einen oder mehrere S&F-Zugänge (SSO) und legst per Haken fest, mit welchen Charakteren du arbeitest. Unter *Charakter wechseln* erscheinen nur diese, jeweils mit Server, Gilde und Konto. Beim Wählen meldet sich der Scanner mit den gespeicherten Daten selbst beim Spiel an; ohne gespeicherte Konten bleibt die einmalige Anmeldung ohne Speichern.

- Die Passwörter liegen nur **verschlüsselt** (AES-256-GCM, gebunden an Benutzer und Konto) in `data/users/<id>/accounts.json`, werden nie wieder angezeigt und tauchen in keiner Antwort oder im Log auf.
- Der Schlüssel kommt aus der Umgebungsvariable **`SFGS_KEY`** (mindestens 32 zufällige Zeichen, z.B. `openssl rand -base64 32`). Ohne Schlüssel ist das Speichern gesperrt. **Den Schlüssel sichern und nicht ändern:** Mit einem anderen Schlüssel lassen sich gespeicherte Passwörter nicht mehr entschlüsseln, die Konten müssen dann neu hinterlegt werden.
- Wer Zugriff auf Schlüssel *und* Datenordner hat, kommt an die Passwörter. Den Schlüssel deshalb nicht im selben Backup wie `data/` ablegen.

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

- S&F-Passwörter werden nur verschlüsselt gespeichert (siehe *S&F-Konten speichern*); bei der einmaligen Anmeldung ohne Speichern bleiben sie nur im RAM
- Der Container braucht Zugang zu `sfgame.net` und `sso.playa-games.com`
- Erster Build dauert 3-5 Min. (Rust kompiliert alle Dependencies)
- Zwischen API-Requests wird 400-500ms Pause gehalten (Rate-Limiting)

## Credits

- [sf-api](https://github.com/the-marenga/sf-api) by [the-marenga](https://github.com/the-marenga) — Rust library for the Shakes & Fidget API

---

## Mitwirkende

Dieses Projekt wurde in Zusammenarbeit mit [Claude](https://claude.ai) (Sonnet 4.6) von [Anthropic](https://anthropic.com) entwickelt und iterativ ausgebaut.  
Der øberwiegende Teil des Codes, der Architektur und der Dokumentation wurde durch KI generiert und gemeinsam verfeinert.

| Rolle | Person / Tool |
|---|---|
| Projektidee, Anforderungen & Tests | [DasAoD](https://git.uliana.de/DasAoD) |
| Code, Architektur, Dokumentation | [Claude](https://git.uliana.de/Claude) (Anthropic) |

## License

[MIT](LICENSE)
