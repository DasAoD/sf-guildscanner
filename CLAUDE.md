# sf-guildscanner – Projektkontext für Claude Code

- S&F-Zugangsdaten nur im RAM halten, nie speichern.
- Alle `/api/*`-Endpunkte außer `auth/*` und `version` brauchen eine Sitzung (`Ctx`-Extractor in `main.rs`, Logik in `auth.rs`). Jeder Benutzer hat eigenen `AppHandles`-Zustand und Datenordner `data/users/<id>/`; neue Handler nehmen `Ctx(handles, user)`, nie einen globalen Zustand.
- Pausen zwischen API-Requests beibehalten: 250 ms (HoF-Seiten), 400 ms (ViewGuild), 700 ms (ViewPlayer in der Simulation).
- `AppState` ist während jedes Spielserver-Requests gesperrt, weil die `SimpleSession` darin liegt. Fortschritts- und Abbruch-Felder gehören in `ProgressState` (`AppHandles`), nicht nach `AppState`, sonst hängen `/api/progress` und die Abbruch-Endpunkte hinter dem laufenden Request.
- Die angezeigte Version kommt aus `APP_VERSION` (Docker-Build-Arg `VERSION`, `/api/version`), nicht aus `Cargo.toml`.
- Kampfsimulation: Angreifer sind nur kampfbereite Mitglieder (<24h online UND für den Angriff angemeldet); Eingeladene (`GuildRank::Invited`) zählen nie.
- Pilzkatapult-Simulation: Ziele werden pro Einzelkampf neu gewürfelt, deshalb Loop mit `iterations=1` statt eines `simulate_battle`-Aufrufs.
- Tag-Format: `vX.Y.Z`
