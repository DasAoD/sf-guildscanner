#![allow(deprecated)]
//! guild_probe — Wegwerf-Testbinary: gibt die Mitglieder der eigenen Gilde
//! (und optional einer fremden) roh aus sf-api aus: Gildenrang (inkl.
//! Eingeladene), Level, Zeit seit letzter Aktivität und angemeldete Kämpfe.
//! Dient der Klärung, wie Eingeladene und frisch Beigetretene aussehen.
//! Kein Teil des eigentlichen Servers.
//!
//! Umgebungsvariablen:
//!   SSO_USERNAME   — Pflicht
//!   SSO_PASSWORD   / PASSWORD — Pflicht
//!   SERVER_HOST    — z.B. f29.sfgame.net (Pflicht)
//!   CHARACTER      — eigener Charaktername (Pflicht)
//!   ENEMY_GUILD    — Name einer fremden Gilde (optional)

use std::env;

use chrono::Local;
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

fn age(ts: Option<chrono::DateTime<Local>>) -> String {
    match ts {
        Some(t) => format!("{:.1} h", Local::now().signed_duration_since(t).num_seconds() as f64 / 3600.0),
        None => "unbekannt".to_string(),
    }
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();

    let sso_user = need_env("SSO_USERNAME");
    let sso_pass = env::var("SSO_PASSWORD")
        .or_else(|_| env::var("PASSWORD"))
        .unwrap_or_else(|_| {
            eprintln!("Fehlende SSO_PASSWORD/PASSWORD");
            std::process::exit(2);
        });
    let server_host = need_env("SERVER_HOST");
    let character = need_env("CHARACTER");
    let enemy = opt_env("ENEMY_GUILD");

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
    let guild = match gs.guild.as_ref() {
        Some(g) => g,
        None => {
            eprintln!("Charakter ist in keiner Gilde");
            std::process::exit(1);
        }
    };

    println!("Eigene Gilde: {} (Rang #{}, Ehre {}), {} Einträge", guild.name, guild.rank, guild.honor, guild.members.len());
    for m in &guild.members {
        println!(
            "  {:<20} Lvl {:>3}  Rang {:?}  zuletzt online vor {}  Kämpfe {:?}",
            m.name,
            m.level,
            m.guild_rank,
            age(m.last_online),
            m.battles_joined
        );
    }

    if let Some(name) = enemy {
        let gs = match session.send_command(Command::ViewGuild { guild_ident: name.clone() }).await {
            Ok(gs) => gs,
            Err(e) => {
                eprintln!("ViewGuild fehlgeschlagen: {e:?}");
                std::process::exit(1);
            }
        };
        match gs.lookup.guilds.get(&name) {
            Some(g) => {
                println!("Fremde Gilde: {} (Rang #{}, Ehre {}), {} Einträge", g.name, g.rank, g.honor, g.members.len());
                for m in &g.members {
                    println!(
                        "  {:<20} Lvl {:>3}  Rang {:?}  zuletzt aktiv vor {}",
                        m.name,
                        m.level,
                        m.rank,
                        age(m.last_active)
                    );
                }
            }
            None => eprintln!("Gilde '{name}' nicht in der Antwort gefunden"),
        }
    }
}
