# sf-guildscanner – Projektkontext für Claude Code

Findet angreifbare Gilden in Shakes & Fidget – basierend auf Mitgliederanzahl und den einzelnen Leveln der Mitglieder.

## Tech-Stack
- Rust (Backend: Axum) + sf-api (the-marenga/sf-api) für die S&F-API-Anbindung
- Web-Frontend: statisches HTML unter `static/`
- Docker (Multi-Stage-Build), Docker Hub: `dasaod/sf-guildscanner`

## Struktur
```
sf-guildscanner/
├── Cargo.toml
├── Dockerfile              # Multi-Stage Build
├── docker-compose.yml
├── src/
│   └── main.rs             # Rust Backend (Axum + sf-api)
├── static/
│   └── index.html          # Web-Frontend
└── data/                   # Persistente Scan-Daten (Volume)
    └── scan_*.json
```

## Funktionsweise
1. Login per SSO → Charakter auswählen → Gildendaten laden
2. Phase 1 – HoF-Scan: scannt komplette Gilden-Ehrenhalle, ermittelt Rang/Ehre/Mitgliederzahl aller Gilden
3. Automatische Filterung nach Spielregel: Gilden unter euch (höherer Rang) sind angreifbar; Gilden über euch nur wenn max. 20 Ränge ODER max. 3000 Ehre Unterschied
4. Phase 2 – Detail-Scan: `ViewGuild` pro angreifbarer Gilde → individuelle Mitglieder-Level
5. Ergebnisse werden als JSON persistiert (`data/scan_*.json`)

## Wichtige Hinweise
- **Login-Daten werden nur im RAM gehalten, nie gespeichert**
- Container braucht Zugang zu `sfgame.net` und `sso.playa-games.com`
- Zwischen API-Requests: 400–500ms Pause (Rate-Limiting) – bei Änderungen am Scan-Loop unbedingt beibehalten
- Erster Build dauert 3–5 Min. (Rust kompiliert alle Dependencies)

## Deployment
```bash
mkdir -p /mnt/user/appdata/sfguild-scanner
cd /mnt/user/appdata/sfguild-scanner
curl -O https://raw.githubusercontent.com/DasAoD/sf-guildscanner/main/docker-compose.yml
docker compose up -d
# Öffnen: http://<HOST-IP>:8085
```
Aus Quellcode bauen: `docker compose -f docker-compose.build.yml up -d --build`

## Scan-Dauer (Referenz für Timeouts/UX)
- Phase 1 (HoF): ~10–40 Sek. (abhängig von Servergröße)
- Phase 2 (Details): ~0,5 Sek. pro Gilde → bei 100 angreifbaren Gilden ca. 1 Minute
- Fortschritt wird live im Browser angezeigt
