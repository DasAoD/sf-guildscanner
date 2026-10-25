//! Gespeicherte S&F-Konten eines Benutzers (`data/users/<id>/accounts.json`).
//!
//! Das SSO-Passwort wird mit AES-256-GCM verschlüsselt abgelegt. Der Schlüssel
//! kommt aus der Umgebungsvariable `SFGS_KEY`; ohne ihn lassen sich keine Konten
//! speichern. Passwörter verlassen den Server nie in Richtung Browser.

use std::io::Write;
use std::sync::Mutex;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Kürzeste erlaubte Schlüsselzeichenfolge: Der Schlüssel wird nur gehasht,
/// nicht gestreckt, deshalb muss sie lang und zufällig sein.
const MIN_KEY_LEN: usize = 32;

/// Serialisiert Lesen-Ändern-Schreiben der Konto-Dateien (kein `.await` darunter).
static FILE_LOCK: Mutex<()> = Mutex::new(());

pub struct Crypto {
    cipher: Aes256Gcm,
}

impl Crypto {
    /// Aus `SFGS_KEY`; `None`, wenn nicht gesetzt oder zu kurz.
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var("SFGS_KEY").ok()?;
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        if raw.chars().count() < MIN_KEY_LEN {
            log::warn!("SFGS_KEY ist kürzer als {} Zeichen und wird ignoriert (z.B. mit `openssl rand -base64 32` erzeugen)", MIN_KEY_LEN);
            return None;
        }
        Some(Self::from_secret(raw))
    }

    fn from_secret(secret: &str) -> Self {
        let key = Sha256::digest(format!("sfgs-accounts-v1:{}", secret).as_bytes());
        Self { cipher: Aes256Gcm::new_from_slice(&key).expect("32-Byte-Schlüssel") }
    }

    /// Verschlüsselt `plain`; `aad` bindet den Text an Benutzer und Konto
    /// (ein vertauschter Eintrag lässt sich nicht entschlüsseln).
    pub fn encrypt(&self, aad: &str, plain: &str) -> Result<String, String> {
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let ct = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: plain.as_bytes(), aad: aad.as_bytes() })
            .map_err(|_| "Verschlüsselung fehlgeschlagen".to_string())?;
        Ok(format!("{}{}", hex(&nonce), hex(&ct)))
    }

    pub fn decrypt(&self, aad: &str, blob: &str) -> Result<String, String> {
        let bytes = unhex(blob).ok_or("Gespeicherte Zugangsdaten sind beschädigt")?;
        if bytes.len() < 12 {
            return Err("Gespeicherte Zugangsdaten sind beschädigt".into());
        }
        let (nonce, ct) = bytes.split_at(12);
        let plain = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad: aad.as_bytes() })
            .map_err(|_| "Zugangsdaten lassen sich nicht entschlüsseln (SFGS_KEY geändert?)".to_string())?;
        String::from_utf8(plain).map_err(|_| "Gespeicherte Zugangsdaten sind beschädigt".to_string())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

pub fn new_id() -> String {
    let mut b = [0u8; 8];
    OsRng.fill_bytes(&mut b);
    format!("a{}", hex(&b))
}

#[derive(Serialize, Deserialize, Clone)]
pub struct StoredChar {
    pub name: String,
    /// Hostname des Spielservers, z.B. `f29.sfgame.net`
    pub server: String,
    #[serde(default)]
    pub guild: Option<String>,
    #[serde(default)]
    pub active: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Account {
    pub id: String,
    pub sso_user: String,
    /// Verschlüsseltes SSO-Passwort
    secret: String,
    pub characters: Vec<StoredChar>,
}

impl Account {
    pub fn new(id: String, sso_user: String, secret: String, characters: Vec<StoredChar>) -> Self {
        Self { id, sso_user, secret, characters }
    }

    pub fn secret(&self) -> &str {
        &self.secret
    }

    pub fn set_secret(&mut self, secret: String) {
        self.secret = secret;
    }
}

/// Was der Browser sehen darf: alles außer dem Passwort.
#[derive(Serialize)]
pub struct AccountView {
    pub id: String,
    pub sso_user: String,
    pub characters: Vec<StoredChar>,
}

impl From<&Account> for AccountView {
    fn from(a: &Account) -> Self {
        Self { id: a.id.clone(), sso_user: a.sso_user.clone(), characters: a.characters.clone() }
    }
}

/// AAD für die Verschlüsselung eines Kontos
pub fn aad(user_id: &str, account_id: &str) -> String {
    format!("{}:{}", user_id, account_id)
}

fn path(dir: &str) -> String {
    format!("{}/accounts.json", dir)
}

/// Fehlende Datei = keine Konten; beschädigte Datei = Fehler (nichts überschreiben).
pub fn load(dir: &str) -> Result<Vec<Account>, String> {
    match std::fs::read_to_string(path(dir)) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("accounts.json ist beschädigt: {}", e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("accounts.json ist nicht lesbar: {}", e)),
    }
}

fn save_unlocked(dir: &str, accounts: &[Account]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(accounts).map_err(|e| e.to_string())?;
    let tmp = format!("{}.tmp", path(dir));
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
    std::fs::rename(&tmp, path(dir)).map_err(|e| e.to_string())
}

/// Konten lesen, ändern und atomar zurückschreiben. `f` darf nicht blockieren
/// oder `.await`en.
pub fn update<T>(dir: &str, f: impl FnOnce(&mut Vec<Account>) -> Result<T, String>) -> Result<T, String> {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut accounts = load(dir)?;
    let out = f(&mut accounts)?;
    save_unlocked(dir, &accounts)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crypto() -> Crypto {
        Crypto::from_secret("ein-langer-zufaelliger-schluessel-fuer-tests-123456")
    }

    fn temp_dir() -> String {
        std::env::temp_dir()
            .join(format!("sfgs-acc-{}", new_id()))
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn encrypt_roundtrip_and_binding() {
        let c = crypto();
        let blob = c.encrypt("u1:a1", "geheimes passwort").unwrap();
        assert!(!blob.contains("geheimes"));
        assert_eq!(c.decrypt("u1:a1", &blob).unwrap(), "geheimes passwort");
        // anderes Konto/anderer Benutzer: nicht entschlüsselbar
        assert!(c.decrypt("u2:a1", &blob).is_err());
        assert!(c.decrypt("u1:a2", &blob).is_err());
        // anderer Schlüssel
        let other = Crypto::from_secret("ein-ganz-anderer-langer-schluessel-fuer-tests-654321");
        assert!(other.decrypt("u1:a1", &blob).is_err());
    }

    #[test]
    fn nonce_differs_per_encryption() {
        let c = crypto();
        assert_ne!(c.encrypt("x", "gleich").unwrap(), c.encrypt("x", "gleich").unwrap());
    }

    #[test]
    fn rejects_damaged_blobs() {
        let c = crypto();
        assert!(c.decrypt("x", "zz").is_err());
        assert!(c.decrypt("x", "abc").is_err());
        assert!(c.decrypt("x", "00ff").is_err());
        let mut blob = c.encrypt("x", "passwort").unwrap();
        blob.replace_range(30..32, "00");
        assert!(c.decrypt("x", &blob).is_err());
    }

    #[test]
    fn short_key_is_ignored() {
        std::env::set_var("SFGS_KEY", "zu kurz");
        assert!(Crypto::from_env().is_none());
        std::env::remove_var("SFGS_KEY");
        assert!(Crypto::from_env().is_none());
    }

    #[test]
    fn store_roundtrip_without_plaintext() {
        let dir = temp_dir();
        let c = crypto();
        update(&dir, |a| {
            a.push(Account::new(
                "a1".into(),
                "daniele".into(),
                c.encrypt(&aad("u1", "a1"), "mein-sso-passwort").unwrap(),
                vec![StoredChar { name: "Medea".into(), server: "f29.sfgame.net".into(), guild: None, active: true }],
            ));
            Ok(())
        })
        .unwrap();
        let raw = std::fs::read_to_string(path(&dir)).unwrap();
        assert!(!raw.contains("mein-sso-passwort"));
        let loaded = load(&dir).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(c.decrypt(&aad("u1", "a1"), loaded[0].secret()).unwrap(), "mein-sso-passwort");
        let view = serde_json::to_string(&AccountView::from(&loaded[0])).unwrap();
        assert!(!view.contains("secret"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_file_is_not_overwritten() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(path(&dir), "{kaputt").unwrap();
        assert!(update(&dir, |a| { a.clear(); Ok(()) }).is_err());
        assert_eq!(std::fs::read_to_string(path(&dir)).unwrap(), "{kaputt");
        std::fs::remove_dir_all(&dir).ok();
    }
}
