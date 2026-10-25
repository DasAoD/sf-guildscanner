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
│   └── main.rs             # Rust Backend (Axum + sf-api): Scan, Simulation, Caches
├── static/
│   └── index.html          # Web-Frontend
├── examples/
│   └── sim_probe.rs        # Wegwerf-Binary: Simulation gegen echte Kämpfe validieren
└── data/                   # Persistente Scan-Daten (Volume)
    ├── scan_*.json         # letzter Scan pro Server/Gilde
    └── history/            # ältere Scans mit Zeitstempel
```

## Funktionsweise
1. Login per SSO → Charakter auswählen → Gildendaten laden. Charakterwechsel geht ohne Neu-Login (alle Sessions bleiben im RAM); dabei werden Scan-Daten, Fortschritt und beide Kampfwerte-Caches verworfen
2. Phase 1 – HoF-Scan: Rangfenster (20 Ränge über uns bis `down_limit` darunter), ermittelt Rang/Ehre/Mitgliederzahl. Zusätzlich je nach `ScanSettings`:
   - `honor_up_scan`: weiter nach oben, ehre-gefiltert, begrenzt durch `max_extra_up_pages`
   - `full_up_scan`: komplette Ehrenhalle bis Rang 1, ohne Ehre-Filter und ohne Seitengrenze (überschreibt `honor_up_scan`). Liegt Seite 0 schon im Rangfenster, läuft die Zusatzphase nicht an; der Rangfilter in Phase 1a hat dafür eine eigene `full_up_scan`-Ausnahme
3. Angreifbarkeit (`is_attackable`): Gilden unter uns immer; Gilden über uns nur bei max. 20 Rängen ODER max. 3000 Ehre Unterschied
4. Phase 2 – Detail-Scan: `ViewGuild` pro angreifbarer Gilde → individuelle Mitglieder-Level
5. Ergebnisse werden als JSON persistiert (`data/scan_*.json`, Verlauf in `data/history/`)
6. Kampfsimulation (`/api/simulate`): per Hintergrund-Task, auch für nicht angreifbare Gilden aus der vollständigen Ehrenhalle-Ansicht

## Kampfsimulation
- Angreifende Seite: nur aktive Mitglieder (<24h offline), Verteidiger: komplette Gilde. Aufstellung aufsteigend nach Level (Last-Man-Standing)
- Pilzkatapult (0–3 Pilze): pro Pilz ein zufälliges Ziel, 50 Prozentpunkte des ursprünglichen Lebens additiv (2 Treffer = 0 %). Ziele werden pro Einzelkampf neu gewürfelt, deshalb Loop mit `iterations=1` statt eines `simulate_battle`-Aufrufs
- 2500 Iterationen als Default (wie sf-tools), `mushrooms` und `iterations` werden geclampt
- Kampfwerte kommen per `ViewPlayer` (700 ms Pause, `SIM_VIEWPLAYER_DELAY_MS`) und werden gecacht: `own_fighters_cache` (ungültig bei Roster-Änderung, nach 6 h oder Charakterwechsel) und `enemy_fighters_cache` pro Gildenname (6 h, max. 100 Einträge)

## Locking
- `AppState` (Sessions, Scan-Daten, Caches) ist für die gesamte Dauer jedes Spielserver-Requests gesperrt, weil die `SimpleSession` darin liegt
- Fortschritt und Abbruch-Flags liegen bewusst in einem eigenen Lock (`ProgressState`, beide gebündelt als `AppHandles`), sonst hängen `/api/progress` und die Abbruch-Endpunkte hinter dem laufenden Spielserver-Request. Neue Fortschrittsfelder dorthin, nicht nach `AppState`

## Version
- Die angezeigte Version kommt aus `APP_VERSION` (Docker-Build-Arg `VERSION`, Endpoint `/api/version`), nicht aus `Cargo.toml` (steht auf 0.2.3)

## Wichtige Hinweise
- **Login-Daten werden nur im RAM gehalten, nie gespeichert**
- Container braucht Zugang zu `sfgame.net` und `sso.playa-games.com`
- Zwischen API-Requests: 250 ms (HoF-Seiten), 400 ms (ViewGuild), 700 ms (ViewPlayer in der Simulation) – bei Änderungen an den Loops unbedingt beibehalten
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
- Simulation: Dauer dominiert vom ViewPlayer-Aufbau der Kampfwerte (~0,7 Sek. pro Mitglied), danach durch den Cache schnell
- Fortschritt wird live im Browser angezeigt
