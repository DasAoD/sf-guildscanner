#![allow(deprecated)]
//! hof_probe — Wegwerf-Testbinary: holt Ehrenhalle-Gildenseiten roh über
//! sf-api und zeigt pro Seite Anzahl, Rangbereich und Rang-Lücken. Dient der
//! Suche nach Gilden, die im Scanner fehlen (z.B. #102 auf einer
//! Seitengrenze). sf-api-Parserwarnungen (verworfene Einträge) werden
//! ausgegeben. Kein Teil des eigentlichen Servers.
//!
//! Umgebungsvariablen:
//!   SSO_USERNAME   — Pflicht
//!   SSO_PASSWORD   / PASSWORD — Pflicht
//!   SERVER_HOST    — z.B. s7.sfgame.eu (Pflicht)
//!   CHARACTER      — eigener Charaktername (Pflicht)
//!   FIRST_PAGE     — erste Seite (optional, default 0)
//!   LAST_PAGE      — letzte Seite (optional, default 3)
//!   STRIDE         — Positionsschritt pro Seite (optional, default 50;
//!                    51 = Verhalten von sf-api's HallOfFameGroupPage, bei dem
//!                    der letzte Rang jeder Seite fehlt)
//!   FIND           — Gildenname, der in den Seiten gesucht wird (optional)

use std::{env, time::Duration};

use sf_api::{command::Command, session::SimpleSession};

fn need_env(key: &str) -> String {
    env::var(key).unwrap_or_else(|_| {
        eprintln!("Fehlende Umgebungsvariable: {key}");
        std::process::exit(2);
    })
}

fn opt_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.trim().is_empty())
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let sso_user = need_env("SSO_USERNAME");
    let sso_pass = env::var("SSO_PASSWORD")
        .or_else(|_| env::var("PASSWORD"))
        .unwrap_or_else(|_| {
            eprintln!("Fehlende SSO_PASSWORD/PASSWORD");
            std::process::exit(2);
        });
    let server_host = need_env("SERVER_HOST");
    let character = need_env("CHARACTER");
    let first: u32 = opt_env("FIRST_PAGE").and_then(|v| v.parse().ok()).unwrap_or(0);
    let last: u32 = opt_env("LAST_PAGE").and_then(|v| v.parse().ok()).unwrap_or(3);
    let find = opt_env("FIND");
    let stride: u32 = opt_env("STRIDE").and_then(|v| v.parse().ok()).unwrap_or(50);

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

    let mut all_ranks: Vec<u32> = Vec::new();
    for page in first..=last {
        let cmd = Command::Custom {
            cmd_name: "GroupGetHallOfFame".to_string(),
            arguments: vec![
                (26 + stride * page).to_string(),
                String::new(),
                "25".to_string(),
                "25".to_string(),
            ],
        };
        let gs = match session.send_command(cmd).await {
            Ok(gs) => gs,
            Err(e) => {
                eprintln!("Seite {page}: Fehler {e:?}");
                continue;
            }
        };
        let guilds = &gs.hall_of_fames.guilds;
        let min = guilds.iter().map(|g| g.rank).min();
        let max = guilds.iter().map(|g| g.rank).max();
        println!(
            "Seite {page}: {} Einträge, Ränge {:?}..{:?}, guilds_total={:?}",
            guilds.len(),
            min,
            max,
            gs.hall_of_fames.guilds_total
        );
        for g in guilds {
            all_ranks.push(g.rank);
            if find.as_deref().is_some_and(|f| g.name == f) {
                println!(
                    "  GEFUNDEN: #{} {} (Leiter {}, {} Mitglieder, Ehre {})",
                    g.rank, g.name, g.leader, g.member_count, g.honor
                );
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    all_ranks.sort_unstable();
    let mut gaps = Vec::new();
    for w in all_ranks.windows(2) {
        for r in (w[0] + 1)..w[1] {
            gaps.push(r);
        }
    }
    let dups = all_ranks.windows(2).filter(|w| w[0] == w[1]).count();
    println!("Fehlende Ränge zwischen erstem und letztem: {gaps:?}");
    println!("Doppelte Ränge: {dups}");
}
