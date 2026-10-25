//! Benutzer, Passwörter und Sitzungen des Scanners selbst (nicht der S&F-Login).
//!
//! Benutzer liegen in `users.json`, Sitzungen in `sessions.json` (nur als
//! SHA-256-Hash des Tokens). Passwörter werden mit Argon2id gehasht.

use std::collections::HashMap;
use std::io::Write;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Semaphore, SemaphorePermit};

pub const COOKIE_NAME: &str = "sfgs_session";
const SESSION_SECS: i64 = 30 * 24 * 3600;
/// Auch eine ständig genutzte Sitzung endet spätestens nach dieser Zeit
const SESSION_MAX_AGE_SECS: i64 = 90 * 24 * 3600;
/// Wie oft die verlängerte Sitzung höchstens auf Platte geschrieben wird
const PERSIST_EVERY_SECS: i64 = 3600;
const MIN_PASSWORD_LEN: usize = 10;
const MAX_PASSWORD_LEN: usize = 128;
const FREE_FAILS: u32 = 3;
const MAX_DELAY_SECS: u64 = 900;
/// Nach dieser Ruhezeit ohne Versuch beginnt die Zählung neu
const DECAY: Duration = Duration::from_secs(3600);
const MAX_TRACKED: usize = 10_000;
/// Gleichzeitige Argon2-Berechnungen (je etwa 19 MiB Arbeitsspeicher)
const HASH_SLOTS: usize = 2;
/// Wie lange eine Anfrage auf einen freien Argon2-Platz wartet
const SLOT_WAIT: Duration = Duration::from_secs(5);

const COMMON_PASSWORDS: &[&str] = &[
    "passwort123", "password123", "1234567890", "qwertz12345", "qwerty12345",
    "willkommen1", "passwort12", "password12", "123456789a", "abcdefghij",
];

#[derive(Serialize, Deserialize, Clone)]
struct UserRec {
    id: String,
    name: String,
    is_admin: bool,
    pw_hash: String,
    created: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct SessionRec {
    token_hash: String,
    user_id: String,
    expires: i64,
    /// Erstellungszeit (Unix-Sekunden)
    #[serde(default)]
    created: i64,
}

#[derive(Serialize, Clone)]
pub struct AuthUser {
    pub id: String,
    pub name: String,
    pub is_admin: bool,
}

#[derive(Serialize)]
pub struct UserInfo {
    pub id: String,
    pub name: String,
    pub is_admin: bool,
    pub created: String,
}

pub enum LoginError {
    Invalid,
    /// Wartezeit in Sekunden
    Throttled(u64),
}

struct Fail {
    count: u32,
    until: Instant,
    last: Instant,
}

pub struct Auth {
    dir: String,
    users: Mutex<Vec<UserRec>>,
    sessions: Mutex<Vec<SessionRec>>,
    last_persist: Mutex<i64>,
    fails: Mutex<HashMap<String, Fail>>,
    slots: Semaphore,
    dummy_hash: String,
    pub cookie_secure: bool,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn argon2() -> Argon2<'static> {
    // OWASP-Mindestwerte für Argon2id: 19 MiB, 2 Durchgänge, 1 Lane
    let params = Params::new(19 * 1024, 2, 1, None).expect("gültige Argon2-Parameter");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn hash_blocking(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    argon2()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

fn verify_blocking(hash: &str, password: &str) -> bool {
    match PasswordHash::new(hash) {
        // Verifizierung nutzt die im Hash gespeicherten Parameter
        Ok(parsed) => Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{:02x}", b)).collect()
}

/// Wartezeit nach `fails` Fehlversuchen: die ersten drei frei, danach
/// 1, 2, 4 ... Sekunden bis höchstens 15 Minuten.
fn delay_for(fails: u32) -> u64 {
    if fails <= FREE_FAILS {
        return 0;
    }
    (1u64 << (fails - FREE_FAILS - 1).min(20)).min(MAX_DELAY_SECS)
}

pub fn check_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    let len = name.chars().count();
    if !(3..=32).contains(&len) {
        return Err("Der Benutzername braucht 3 bis 32 Zeichen".into());
    }
    if !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.')) {
        return Err("Erlaubt sind Buchstaben, Ziffern, _ - und .".into());
    }
    Ok(name.to_string())
}

pub fn check_password(name: &str, password: &str) -> Result<(), String> {
    let len = password.chars().count();
    if len < MIN_PASSWORD_LEN {
        return Err(format!("Das Passwort braucht mindestens {} Zeichen", MIN_PASSWORD_LEN));
    }
    if len > MAX_PASSWORD_LEN {
        return Err(format!("Das Passwort darf höchstens {} Zeichen haben", MAX_PASSWORD_LEN));
    }
    let lower = password.to_lowercase();
    if lower == name.to_lowercase() || COMMON_PASSWORDS.contains(&lower.as_str()) {
        return Err("Dieses Passwort ist zu einfach".into());
    }
    Ok(())
}

/// Sitzungs-Token aus dem Cookie-Header lesen
pub fn token_from_cookie_header(header: &str) -> Option<String> {
    header
        .split(';')
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE_NAME)
        .map(|(_, v)| v.to_string())
        .filter(|v| !v.is_empty())
}

/// Schreibt über eine Temp-Datei, damit ein Abbruch die alte Datei nicht
/// halb überschreibt. Die Datei ist von Anfang an nur für den Besitzer lesbar.
fn write_atomic(path: &str, json: &str) -> Result<(), String> {
    let tmp = format!("{}.tmp", path);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&tmp).map_err(|e| e.to_string())?;
    file.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Fehlende Datei = leer. Jeder andere Fehler bricht den Start ab: Eine
/// beschädigte `users.json` als leer zu lesen würde die Einrichtung für den
/// Nächstbesten öffnen.
fn load_users(path: &str) -> Vec<UserRec> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s)
            .unwrap_or_else(|e| panic!("{} ist beschädigt: {}. Bitte prüfen oder sichern; der Scanner startet absichtlich nicht.", path, e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => panic!("{} ist nicht lesbar: {}. Der Scanner startet absichtlich nicht.", path, e),
    }
}

impl Auth {
    pub fn new(dir: &str, cookie_secure: bool) -> Self {
        let users = load_users(&format!("{}/users.json", dir));
        let mut sessions: Vec<SessionRec> = std::fs::read_to_string(format!("{}/sessions.json", dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let t = now();
        sessions.retain(|s| s.expires > t);

        Self {
            dir: dir.to_string(),
            users: Mutex::new(users),
            sessions: Mutex::new(sessions),
            last_persist: Mutex::new(t),
            fails: Mutex::new(HashMap::new()),
            slots: Semaphore::new(HASH_SLOTS),
            // Für unbekannte Benutzer wird trotzdem gerechnet, damit die
            // Antwortzeit nicht verrät, ob es den Namen gibt.
            dummy_hash: hash_blocking("dummy-passwort-fuer-gleiche-laufzeit").unwrap_or_default(),
            cookie_secure,
        }
    }

    fn persist_users(&self, users: &[UserRec]) -> Result<(), String> {
        let json = serde_json::to_string_pretty(users).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        write_atomic(&format!("{}/users.json", self.dir), &json)
            .map_err(|e| format!("Speichern fehlgeschlagen: {}", e))
    }

    fn persist_sessions(&self, sessions: &[SessionRec]) {
        let result = serde_json::to_string(sessions)
            .map_err(|e| e.to_string())
            .and_then(|j| {
                std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
                write_atomic(&format!("{}/sessions.json", self.dir), &j)
            });
        if let Err(e) = result {
            log::error!("sessions.json konnte nicht geschrieben werden: {}", e);
        }
        *self.last_persist.lock().unwrap() = now();
    }

    /// Wartet kurz auf einen freien Argon2-Platz; bei Überlast `None`.
    async fn slot(&self) -> Option<SemaphorePermit<'_>> {
        tokio::time::timeout(SLOT_WAIT, self.slots.acquire()).await.ok()?.ok()
    }

    async fn hash(&self, password: &str) -> Result<String, String> {
        let _permit = self.slot().await.ok_or("Der Server ist gerade ausgelastet, bitte gleich nochmal versuchen")?;
        let pw = password.to_string();
        tokio::task::spawn_blocking(move || hash_blocking(&pw))
            .await
            .map_err(|e| e.to_string())?
    }

    /// Bei Überlast zählt die Prüfung als fehlgeschlagen.
    async fn verify(&self, hash: &str, password: &str) -> bool {
        let Some(_permit) = self.slot().await else {
            return false;
        };
        let (h, pw) = (hash.to_string(), password.to_string());
        tokio::task::spawn_blocking(move || verify_blocking(&h, &pw))
            .await
            .unwrap_or(false)
    }

    pub fn needs_setup(&self) -> bool {
        self.users.lock().unwrap().is_empty()
    }

    fn new_session(&self, user_id: &str) -> String {
        let token = random_hex(32);
        let mut sessions = self.sessions.lock().unwrap();
        let t = now();
        sessions.retain(|s| s.expires > t);
        sessions.push(SessionRec {
            token_hash: token_hash(&token),
            user_id: user_id.to_string(),
            expires: t + SESSION_SECS,
            created: t,
        });
        self.persist_sessions(&sessions);
        token
    }

    fn auth_user(rec: &UserRec) -> AuthUser {
        AuthUser { id: rec.id.clone(), name: rec.name.clone(), is_admin: rec.is_admin }
    }

    /// Ersten Admin anlegen; geht nur, solange es noch keinen Benutzer gibt.
    pub async fn setup(&self, name: &str, password: &str) -> Result<(AuthUser, String), String> {
        let name = check_name(name)?;
        check_password(&name, password)?;
        let pw_hash = self.hash(password).await?;
        let rec = {
            let mut users = self.users.lock().unwrap();
            if !users.is_empty() {
                return Err("Die Einrichtung ist bereits abgeschlossen".into());
            }
            let rec = UserRec {
                id: format!("u{}", random_hex(8)),
                name,
                is_admin: true,
                pw_hash,
                created: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            };
            users.push(rec.clone());
            if let Err(e) = self.persist_users(&users) {
                users.pop();
                return Err(e);
            }
            rec
        };
        let token = self.new_session(&rec.id);
        Ok((Self::auth_user(&rec), token))
    }

    /// Versuch vor der Prüfung zählen (nicht erst danach), damit eine Salve
    /// gleichzeitiger Anfragen die Drosselung nicht unterläuft. Gibt bei
    /// laufender Sperre die Wartezeit in Sekunden zurück.
    fn reserve(&self, keys: &[String]) -> Result<(), u64> {
        let mut fails = self.fails.lock().unwrap();
        let t = Instant::now();
        let wait = keys
            .iter()
            .filter_map(|k| fails.get(k))
            .filter(|f| f.until > t)
            .map(|f| (f.until - t).as_secs() + 1)
            .max();
        if let Some(w) = wait {
            return Err(w);
        }
        if fails.len() >= MAX_TRACKED {
            fails.retain(|_, f| t.duration_since(f.last) < DECAY);
        }
        for k in keys {
            // Schlüssel bestehender Benutzer (`name:`, `pw:`) sind durch die Benutzerzahl
            // begrenzt und werden nie übersprungen; Adressen und erfundene Namen nur
            // bis zum Limit.
            let essential = k.starts_with("name:") || k.starts_with("pw:");
            if !essential && !fails.contains_key(k) && fails.len() >= MAX_TRACKED {
                continue;
            }
            let f = fails.entry(k.clone()).or_insert(Fail { count: 0, until: t, last: t });
            if t.duration_since(f.last) >= DECAY {
                f.count = 0;
            }
            f.count += 1;
            f.last = t;
            f.until = t + Duration::from_secs(delay_for(f.count));
        }
        Ok(())
    }

    fn clear_fails(&self, keys: &[String]) {
        let mut fails = self.fails.lock().unwrap();
        for k in keys {
            fails.remove(k);
        }
    }

    /// `ip` nur, wenn dem Reverse-Proxy vertraut wird. Gezählt wird pro Name
    /// (auch erfundene, damit die Antwort nicht verrät, ob es den Benutzer
    /// gibt) und pro Adresse. Ein Erfolg löscht nur den Zähler des Namens, damit
    /// man mit einem eigenen Konto den Adress-Zähler nicht zurücksetzen kann.
    pub async fn login(&self, name: &str, password: &str, ip: Option<&str>) -> Result<(AuthUser, String), LoginError> {
        let rec = {
            let users = self.users.lock().unwrap();
            users.iter().find(|u| u.name.eq_ignore_ascii_case(name.trim())).cloned()
        };

        let name_key = match &rec {
            Some(r) => format!("name:{}", r.name.to_lowercase()),
            None => format!("unk:{}", name.trim().to_lowercase().chars().take(64).collect::<String>()),
        };
        let mut keys = vec![name_key.clone()];
        if let Some(ip) = ip {
            keys.push(format!("ip:{}", ip.chars().take(64).collect::<String>()));
        }
        if let Err(wait) = self.reserve(&keys) {
            return Err(LoginError::Throttled(wait));
        }

        let ok = match &rec {
            Some(r) => self.verify(&r.pw_hash, password).await,
            None => {
                self.verify(&self.dummy_hash, password).await;
                false
            }
        };

        match rec {
            Some(r) if ok => {
                self.clear_fails(&[name_key]);
                let token = self.new_session(&r.id);
                Ok((Self::auth_user(&r), token))
            }
            _ => Err(LoginError::Invalid),
        }
    }

    /// Benutzer zu einem Sitzungs-Token; verlängert die Sitzung bei Nutzung,
    /// aber nie über `SESSION_MAX_AGE_SECS` seit der Anmeldung hinaus.
    pub fn user_for_token(&self, token: &str) -> Option<AuthUser> {
        let th = token_hash(token);
        let t = now();
        let user_id = {
            let mut sessions = self.sessions.lock().unwrap();
            let s = sessions
                .iter_mut()
                .find(|s| s.token_hash == th && s.expires > t && (s.created == 0 || t - s.created < SESSION_MAX_AGE_SECS))?;
            s.expires = t + SESSION_SECS;
            let id = s.user_id.clone();
            let due = t - *self.last_persist.lock().unwrap() > PERSIST_EVERY_SECS;
            if due {
                self.persist_sessions(&sessions);
            }
            id
        };
        let users = self.users.lock().unwrap();
        users.iter().find(|u| u.id == user_id).map(Self::auth_user)
    }

    pub fn logout(&self, token: &str) {
        let th = token_hash(token);
        let mut sessions = self.sessions.lock().unwrap();
        let before = sessions.len();
        sessions.retain(|s| s.token_hash != th);
        if sessions.len() != before {
            self.persist_sessions(&sessions);
        }
    }

    fn revoke_user_sessions(&self, user_id: &str, keep: Option<&str>) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|s| s.user_id != user_id || Some(s.token_hash.as_str()) == keep);
        self.persist_sessions(&sessions);
    }

    /// Passwort-Hash eines Benutzers ersetzen; bei Schreibfehler zurückrollen.
    fn replace_hash(&self, user_id: &str, new_hash: String) -> Result<(), String> {
        let mut users = self.users.lock().unwrap();
        let u = users.iter_mut().find(|u| u.id == user_id).ok_or("Benutzer nicht gefunden")?;
        let old = std::mem::replace(&mut u.pw_hash, new_hash);
        if let Err(e) = self.persist_users(&users) {
            if let Some(u) = users.iter_mut().find(|u| u.id == user_id) {
                u.pw_hash = old;
            }
            return Err(e);
        }
        Ok(())
    }

    /// Eigenes Passwort ändern; andere Sitzungen des Benutzers enden. Das
    /// Raten des bisherigen Passworts wird wie beim Login gebremst.
    pub async fn change_password(&self, user_id: &str, current_token: &str, old: &str, new: &str) -> Result<(), String> {
        let (name, old_hash) = {
            let users = self.users.lock().unwrap();
            let u = users.iter().find(|u| u.id == user_id).ok_or("Benutzer nicht gefunden")?;
            (u.name.clone(), u.pw_hash.clone())
        };
        let key = vec![format!("pw:{}", user_id)];
        if let Err(wait) = self.reserve(&key) {
            return Err(format!("Zu viele Versuche, bitte in {} Sekunden erneut versuchen", wait));
        }
        if !self.verify(&old_hash, old).await {
            return Err("Das bisherige Passwort stimmt nicht".into());
        }
        self.clear_fails(&key);
        check_password(&name, new)?;
        let new_hash = self.hash(new).await?;
        self.replace_hash(user_id, new_hash)?;
        self.revoke_user_sessions(user_id, Some(&token_hash(current_token)));
        Ok(())
    }

    pub fn list_users(&self) -> Vec<UserInfo> {
        self.users
            .lock()
            .unwrap()
            .iter()
            .map(|u| UserInfo { id: u.id.clone(), name: u.name.clone(), is_admin: u.is_admin, created: u.created.clone() })
            .collect()
    }

    pub async fn create_user(&self, name: &str, password: &str, is_admin: bool) -> Result<UserInfo, String> {
        let name = check_name(name)?;
        check_password(&name, password)?;
        let pw_hash = self.hash(password).await?;
        let mut users = self.users.lock().unwrap();
        if users.iter().any(|u| u.name.eq_ignore_ascii_case(&name)) {
            return Err("Diesen Benutzernamen gibt es schon".into());
        }
        let rec = UserRec {
            id: format!("u{}", random_hex(8)),
            name,
            is_admin,
            pw_hash,
            created: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        };
        users.push(rec.clone());
        if let Err(e) = self.persist_users(&users) {
            users.pop();
            return Err(e);
        }
        Ok(UserInfo { id: rec.id, name: rec.name, is_admin: rec.is_admin, created: rec.created })
    }

    /// Benutzer löschen. Man kann sich nicht selbst löschen und der letzte
    /// Admin bleibt immer bestehen.
    pub fn delete_user(&self, id: &str, acting_id: &str) -> Result<(), String> {
        if id == acting_id {
            return Err("Du kannst dich nicht selbst löschen".into());
        }
        {
            let mut users = self.users.lock().unwrap();
            let pos = users.iter().position(|u| u.id == id).ok_or("Benutzer nicht gefunden")?;
            if users[pos].is_admin && users.iter().filter(|u| u.is_admin).count() == 1 {
                return Err("Der letzte Admin kann nicht gelöscht werden".into());
            }
            let removed = users.remove(pos);
            if let Err(e) = self.persist_users(&users) {
                users.insert(pos, removed);
                return Err(e);
            }
        }
        self.revoke_user_sessions(id, None);
        Ok(())
    }

    /// Passwort eines Benutzers setzen (Admin); alle seine Sitzungen enden.
    pub async fn set_password(&self, id: &str, password: &str) -> Result<(), String> {
        let name = {
            let users = self.users.lock().unwrap();
            users.iter().find(|u| u.id == id).ok_or("Benutzer nicht gefunden")?.name.clone()
        };
        check_password(&name, password)?;
        let new_hash = self.hash(password).await?;
        self.replace_hash(id, new_hash)?;
        self.revoke_user_sessions(id, None);
        Ok(())
    }

    pub fn session_cookie(&self, token: &str) -> String {
        format!(
            "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
            COOKIE_NAME,
            token,
            SESSION_SECS,
            if self.cookie_secure { "; Secure" } else { "" }
        )
    }

    pub fn clear_cookie(&self) -> String {
        format!(
            "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
            COOKIE_NAME,
            if self.cookie_secure { "; Secure" } else { "" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> String {
        std::env::temp_dir()
            .join(format!("sfgs-auth-{}", random_hex(4)))
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn password_policy() {
        assert!(check_password("daniele", "kurz").is_err());
        assert!(check_password("daniele", "passwort123").is_err());
        assert!(check_password("daniele", "Daniele").is_err());
        assert!(check_password("daniele", "ein langes, gutes passwort").is_ok());
        assert!(check_password("daniele", &"x".repeat(129)).is_err());
    }

    #[test]
    fn name_policy() {
        assert_eq!(check_name("  DasAoD ").unwrap(), "DasAoD");
        assert!(check_name("ab").is_err());
        assert!(check_name("mit leerzeichen").is_err());
        assert!(check_name("a/b/c").is_err());
        assert!(check_name(&"x".repeat(33)).is_err());
    }

    #[test]
    fn hash_roundtrip() {
        let h = hash_blocking("ein langes, gutes passwort").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_blocking(&h, "ein langes, gutes passwort"));
        assert!(!verify_blocking(&h, "ein langes, falsches passwort"));
        assert!(!verify_blocking("kein-hash", "x"));
    }

    #[test]
    fn throttle_schedule() {
        assert_eq!(delay_for(1), 0);
        assert_eq!(delay_for(3), 0);
        assert_eq!(delay_for(4), 1);
        assert_eq!(delay_for(5), 2);
        assert_eq!(delay_for(6), 4);
        assert_eq!(delay_for(40), MAX_DELAY_SECS);
    }

    #[test]
    fn reserve_blocks_after_free_attempts() {
        let dir = temp_dir();
        let auth = Auth::new(&dir, false);
        let keys = vec!["name:x".to_string()];
        for _ in 0..4 {
            assert!(auth.reserve(&keys).is_ok());
        }
        // Schon der fünfte gleichzeitige Versuch wird abgewiesen, auch bevor
        // einer der vorherigen geprüft wurde.
        assert!(auth.reserve(&keys).is_err());
        auth.clear_fails(&keys);
        assert!(auth.reserve(&keys).is_ok());
    }

    #[test]
    fn cookie_parsing() {
        assert_eq!(token_from_cookie_header("a=1; sfgs_session=abc123; b=2"), Some("abc123".into()));
        assert_eq!(token_from_cookie_header("a=1"), None);
        assert_eq!(token_from_cookie_header("sfgs_session="), None);
    }

    #[test]
    fn tokens_are_random_and_hashed() {
        let (a, b) = (random_hex(32), random_hex(32));
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_ne!(token_hash(&a), a);
        assert_eq!(token_hash(&a), token_hash(&a));
    }

    #[tokio::test]
    async fn setup_login_session_flow() {
        let dir = temp_dir();
        let auth = Auth::new(&dir, false);
        assert!(auth.needs_setup());

        let (admin, token) = auth.setup("Admin1", "ein langes, gutes passwort").await.unwrap();
        assert!(admin.is_admin);
        assert!(!auth.needs_setup());
        assert!(auth.setup("Admin2", "ein langes, gutes passwort").await.is_err());
        assert_eq!(auth.user_for_token(&token).unwrap().id, admin.id);
        assert!(auth.user_for_token("falsch").is_none());

        assert!(matches!(auth.login("admin1", "falsches passwort!!", Some("ip")).await, Err(LoginError::Invalid)));
        let (_, t2) = auth.login("ADMIN1", "ein langes, gutes passwort", Some("ip")).await.ok().unwrap();
        assert!(auth.user_for_token(&t2).is_some());
        auth.logout(&t2);
        assert!(auth.user_for_token(&t2).is_none());

        assert!(auth.delete_user(&admin.id, &admin.id).is_err());
        let user = auth.create_user("Zweiter", "noch ein gutes passwort", false).await.unwrap();
        assert!(auth.create_user("zweiter", "noch ein gutes passwort", false).await.is_err());
        assert!(auth.delete_user(&admin.id, &user.id).is_err());
        assert!(auth.delete_user(&user.id, &admin.id).is_ok());

        let reloaded = Auth::new(&dir, false);
        assert!(reloaded.user_for_token(&token).is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn login_throttle_kicks_in() {
        let dir = temp_dir();
        let auth = Auth::new(&dir, false);
        auth.setup("Admin1", "ein langes, gutes passwort").await.unwrap();
        for _ in 0..4 {
            let _ = auth.login("Admin1", "falsches passwort!!", Some("ip")).await;
        }
        assert!(matches!(
            auth.login("Admin1", "ein langes, gutes passwort", Some("ip")).await,
            Err(LoginError::Throttled(_))
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn unknown_names_are_throttled_like_known_ones() {
        let dir = temp_dir();
        let auth = Auth::new(&dir, false);
        auth.setup("Admin1", "ein langes, gutes passwort").await.unwrap();
        // erfundene Namen werden genauso gebremst wie bestehende, die Antwort
        // verrät also nicht, ob es den Benutzer gibt
        for _ in 0..4 {
            let _ = auth.login("niemand", "falsches passwort!!", None).await;
        }
        assert!(matches!(auth.login("niemand", "falsches passwort!!", None).await, Err(LoginError::Throttled(_))));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn success_clears_only_the_name_counter() {
        let dir = temp_dir();
        let auth = Auth::new(&dir, false);
        auth.setup("Admin1", "ein langes, gutes passwort").await.unwrap();
        for _ in 0..2 {
            let _ = auth.login("niemand", "falsches passwort!!", Some("1.2.3.4")).await;
        }
        let (_, _) = auth.login("Admin1", "ein langes, gutes passwort", Some("1.2.3.4")).await.ok().unwrap();
        let fails = auth.fails.lock().unwrap();
        assert!(fails.contains_key("ip:1.2.3.4"));
        assert!(!fails.contains_key("name:admin1"));
        drop(fails);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn write_failure_rolls_back() {
        let dir = temp_dir();
        let auth = Auth::new(&dir, false);
        auth.setup("Admin1", "ein langes, gutes passwort").await.unwrap();
        // users.json durch ein Verzeichnis ersetzen: Schreiben scheitert
        std::fs::remove_file(format!("{}/users.json", dir)).unwrap();
        std::fs::create_dir(format!("{}/users.json", dir)).unwrap();
        assert!(auth.create_user("Zweiter", "noch ein gutes passwort", false).await.is_err());
        assert_eq!(auth.list_users().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    #[should_panic(expected = "beschädigt")]
    fn corrupt_users_file_aborts_start() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(format!("{}/users.json", dir), "{kaputt").unwrap();
        let _ = Auth::new(&dir, false);
    }
}
