#![allow(deprecated)]
//! sim_probe — Wegwerf-Testbinary für die Gildenkampf-Simulation
//!
//! Lädt die eigenen aktiven Gildenmitglieder (< 1 Tag offline, ohne uns
//! selbst — die eigenen Daten kommen kostenlos aus dem GameState nach dem
//! Login) sowie alle Mitglieder einer per ENEMY_GUILD angegebenen
//! gegnerischen Gilde per ViewPlayer, baut daraus sf-api `Fighter`s und
//! lässt `simulate_battle` laufen. Dient NUR zur Validierung gegen echte,
//! bekannte Kampfausgänge — kein Teil des eigentlichen Servers.
//!
//! Kampfreihenfolge: aufsteigend nach Level (schwächster zuerst), auf
//! beiden Seiten — das entspricht der echten Last-Man-Standing-Regel.
//! Eigene Seite: nur aktive Mitglieder (< 24h offline), da nur die bei
//! einem von uns gestarteten Angriff antreten. Gegner-Seite: alle
//! Mitglieder, da bei einem gegnerischen Angriff immer die komplette
//! Gilde verteidigt (Offline-Status dafür irrelevant).
//!
//! Umgebungsvariablen:
//!   SSO_USERNAME   — Pflicht
//!   SSO_PASSWORD   / PASSWORD — Pflicht
//!   SERVER_HOST    — z.B. f25.sfgame.net (Pflicht)
//!   CHARACTER      — eigener Charaktername (Pflicht)
//!   ENEMY_GUILD    — Name der zu simulierenden gegnerischen Gilde (Pflicht)
//!   ITERATIONS     — Anzahl simulierter Kämpfe (optional, default 500)
//!   DELAY_MS       — Pause zwischen ViewPlayer-Calls (optional, default 700,
//!                    siehe sfguildsv2/character_sync.rs zur Begründung)
//!   TIME_BUDGET_S  — Hartes Zeitbudget für alle ViewPlayer-Calls, um die
//!                    Session nicht durch zu viele Requests zu gefährden
//!                    (optional, default 90)

use std::{env, time::{Duration, Instant}};

use sf_api::{
    command::Command,
    session::SimpleSession,
    simulate::{Fighter, PlayerFighterSquad, UpgradeableFighter, simulate_battle},
};

fn need_env(key: &str) -> String {
    env::var(key).unwrap_or_else(|_| {
        eprintln!("Fehlende Umgebungsvariable: {key}");
        std::process::exit(2);
    })
}

fn opt_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// Holt einen einzelnen Spieler per ViewPlayer und baut daraus einen
/// UpgradeableFighter. Pausiert vor jedem Call außer dem ersten.
async fn fetch_fighter(
    session: &mut SimpleSession,
    name: &str,
    delay_ms: u64,
    calls_made: &mut u32,
) -> Option<UpgradeableFighter> {
    if *calls_made > 0 {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
    *calls_made += 1;
    let gs = session
        .send_command(Command::ViewPlayer { ident: name.to_string() })
        .await
        .ok()?;
    gs.lookup.lookup_name(name).map(UpgradeableFighter::from_other)
}

#[tokio::main]
async fn main() {
    let sso_user = need_env("SSO_USERNAME");
    let sso_pass = env::var("SSO_PASSWORD")
        .or_else(|_| env::var("PASSWORD"))
        .unwrap_or_else(|_| {
            eprintln!("Fehlende SSO_PASSWORD/PASSWORD");
            std::process::exit(2);
        });
    let server_host = need_env("SERVER_HOST");
    let character = need_env("CHARACTER");
    let enemy_guild = need_env("ENEMY_GUILD");
    let iterations: u32 = opt_env("ITERATIONS").and_then(|v| v.parse().ok()).unwrap_or(500);
    let delay_ms: u64 = opt_env("DELAY_MS").and_then(|v| v.parse().ok()).unwrap_or(700);
    let time_budget_s: u64 = opt_env("TIME_BUDGET_S").and_then(|v| v.parse().ok()).unwrap_or(90);
    let budget = Duration::from_secs(time_budget_s);

    // ── Login (gleiches Muster wie sfguildsv2/rust_examples) ──────────────
    let mut sessions = match SimpleSession::login_sf_account(&sso_user, &sso_pass).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SSO Login fehlgeschlagen: {e:?}");
            std::process::exit(1);
        }
    };

    let pos = match sessions.iter().position(|s| {
        s.server_url().host_str().unwrap_or("").contains(&server_host)
            && s.username() == character.as_str()
    }) {
        Some(p) => p,
        None => {
            eprintln!("Charakter '{character}' auf '{server_host}' nicht gefunden");
            std::process::exit(1);
        }
    };
    let mut session = sessions.remove(pos);

    let gs = match session.send_command(Command::Update).await {
        Ok(gs) => gs,
        Err(e) => {
            eprintln!("Update fehlgeschlagen: {e:?}");
            std::process::exit(1);
        }
    };

    let guild = match gs.guild.clone() {
        Some(g) => g,
        None => {
            eprintln!("Charakter ist in keiner Gilde");
            std::process::exit(1);
        }
    };
    let own_name = gs.character.name.clone();

    println!("Eigene Gilde: {} | Rang #{} | Ehre {}", guild.name, guild.rank, guild.honor);

    // ── Eigene Seite: wir selbst (kostenlos aus GameState) + aktive Mitglieder ──
    let squad = PlayerFighterSquad::new(gs);
    let mut own_fighters: Vec<(u16, Fighter)> = vec![
        (squad.character.level, Fighter::from(&squad.character)),
    ];

    let active_members: Vec<String> = guild
        .members
        .iter()
        .filter(|m| {
            if m.name == own_name {
                return false;
            }
            // Nur Mitglieder < 24h offline nehmen an einem von uns
            // gestarteten Angriff teil.
            match m.last_online {
                Some(ts) => chrono::Local::now().signed_duration_since(ts) < chrono::Duration::days(1),
                None => false,
            }
        })
        .map(|m| m.name.clone())
        .collect();

    println!(
        "Eigene aktive Mitglieder (< 24h offline, ohne uns selbst): {}",
        active_members.len()
    );

    let start = Instant::now();
    let mut calls_made = 0u32;

    for name in &active_members {
        if start.elapsed() > budget {
            eprintln!("Zeitbudget erreicht, überspringe restliche eigene Mitglieder");
            break;
        }
        match fetch_fighter(&mut session, name, delay_ms, &mut calls_made).await {
            Some(uf) => own_fighters.push((uf.level, Fighter::from(&uf))),
            None => eprintln!("ViewPlayer für eigenes Mitglied '{name}' fehlgeschlagen, wird ausgelassen"),
        }
    }

    own_fighters.sort_by_key(|(lvl, _)| *lvl);
    println!("Eigene Kampf-Aufstellung (aufsteigend): {} Kämpfer", own_fighters.len());

    // ── Gegner-Seite: komplette Gilde, kein Offline-Filter ─────────────────
    let gs = match session.send_command(Command::ViewGuild { guild_ident: enemy_guild.clone() }).await {
        Ok(gs) => gs,
        Err(e) => {
            eprintln!("ViewGuild für '{enemy_guild}' fehlgeschlagen: {e:?}");
            std::process::exit(1);
        }
    };
    let enemy_members: Vec<String> = match gs.lookup.guilds.get(&enemy_guild) {
        Some(g) => g.members.iter().map(|m| m.name.clone()).collect(),
        None => {
            eprintln!("Gilde '{enemy_guild}' nicht gefunden");
            std::process::exit(1);
        }
    };

    println!("Gegner-Gilde '{enemy_guild}': {} Mitglieder", enemy_members.len());

    let mut enemy_fighters: Vec<(u16, Fighter)> = Vec::new();
    for name in &enemy_members {
        if start.elapsed() > budget {
            eprintln!("Zeitbudget erreicht, überspringe restliche Gegner-Mitglieder");
            break;
        }
        match fetch_fighter(&mut session, name, delay_ms, &mut calls_made).await {
            Some(uf) => enemy_fighters.push((uf.level, Fighter::from(&uf))),
            None => eprintln!("ViewPlayer für Gegner '{name}' fehlgeschlagen, wird ausgelassen"),
        }
    }

    enemy_fighters.sort_by_key(|(lvl, _)| *lvl);
    println!("Gegner-Kampf-Aufstellung (aufsteigend): {} Kämpfer", enemy_fighters.len());

    if own_fighters.is_empty() || enemy_fighters.is_empty() {
        eprintln!("Mindestens eine Seite hat keine Kämpfer, Simulation nicht möglich");
        std::process::exit(1);
    }

    let left: Vec<Fighter> = own_fighters.into_iter().map(|(_, f)| f).collect();
    let right: Vec<Fighter> = enemy_fighters.into_iter().map(|(_, f)| f).collect();

    // is_arena_battle=false: Gildenkämpfe sind kein 1v1-Arena-Duell, sondern
    // ein Last-Man-Standing-Gauntlet — Annahme, gegen echte Ergebnisse zu
    // prüfen.
    let result = simulate_battle(&left, &right, iterations, false);

    println!();
    println!("═══ Ergebnis ({iterations} simulierte Kämpfe) ═══");
    println!(
        "Sieg-Wahrscheinlichkeit: {:.1}% ({} von {})",
        result.win_ratio * 100.0,
        result.won_fights,
        iterations
    );
    println!();
    println!(
        "Zum Abgleich mit echten Kampfaufzeichnungen: Bewerte, ob dieses Ergebnis \
         (klar/knapp/klar dagegen) zum tatsächlichen Ausgang passt."
    );
}
