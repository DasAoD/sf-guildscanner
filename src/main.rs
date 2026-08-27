use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tower_http::services::ServeDir;

use sf_api::{
    command::Command,
    session::SimpleSession,
    simulate::{Fighter, PlayerFighterSquad, UpgradeableFighter, simulate_battle},
};

// ═══════════════════════════════════════════════════════════════════════════════
//  Data Types
// ═══════════════════════════════════════════════════════════════════════════════

/// Persistent scan data that gets saved to JSON
#[derive(Serialize, Deserialize, Clone, Default)]
struct ScanData {
    /// Timestamp of the scan
    scanned_at: String,
    /// Server URL this scan was done on
    server: String,
    /// Info about our own guild
    own_guild: OwnGuild,
    /// All guilds found in the Hall of Fame
    hof_guilds: Vec<HofGuild>,
    /// Guilds with detailed member data (from ViewGuild)
    detailed_guilds: Vec<DetailedGuild>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct OwnGuild {
    name: String,
    rank: u32,
    honor: u32,
    member_count: usize,
    #[serde(default)]
    active_member_count: usize,
    members: Vec<MemberInfo>,
    #[serde(default)]
    active_members: Vec<MemberInfo>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct MemberInfo {
    name: String,
    level: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_online: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offline_days: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_active_24h: Option<bool>,
}

/// Guild as seen in the Hall of Fame (no member details)
#[derive(Serialize, Deserialize, Clone, Default)]
struct HofGuild {
    name: String,
    rank: u32,
    leader: String,
    member_count: u32,
    honor: u32,
    is_attacked: bool,
    /// Calculated: is this guild attackable by us?
    attackable: bool,
}

/// Guild with full member details from ViewGuild
#[derive(Serialize, Deserialize, Clone, Default)]
struct DetailedGuild {
    name: String,
    rank: u32,
    honor: u32,
    member_count: usize,
    members: Vec<MemberInfo>,
    max_level: u16,
    min_level: u16,
    total_level: u32,
    finished_raids: u16,
    is_attacked: bool,
    /// Ist diese Gilde nach der Rang/Ehre-Regel aktuell angreifbar? Normale
    /// Scan-Kandidaten sind das immer (true) — Gilden, die nur zum
    /// Vormerken/Probesimulieren aus der vollständigen Ehrenhalle-Ansicht
    /// hier landen, können false sein.
    #[serde(default = "default_true")]
    attackable: bool,

    // ── Strict mode evaluation (level-to-level, ascending) ─────────────────
    #[serde(default)]
    strict_evaluated: bool,
    #[serde(default)]
    strict_beatable: bool,
    #[serde(default)]
    strict_own_active_members: usize,
    #[serde(default)]
    strict_topn: bool,
    #[serde(default)]
    strict_topn_n: usize,
    #[serde(default)]
    strict_fail_index: Option<usize>,
    #[serde(default)]
    strict_fail_enemy_level: Option<u16>,
    #[serde(default)]
    strict_fail_own_level: Option<u16>,
    #[serde(default)]
    strict_fail_reason: Option<String>,

    // ── Kampf-Simulation (sf_api::simulate, on-demand via /api/simulate) ───
    #[serde(default)]
    sim_win_ratio: Option<f64>,
    #[serde(default)]
    sim_mushrooms: Option<u8>,
    #[serde(default)]
    sim_iterations: Option<u32>,
    #[serde(default)]
    sim_evaluated_at: Option<String>,
}

// ── App State ────────────────────────────────────────────────────────────────

/// Alles, was den Login-/Scan-/Simulations-Datenstand betrifft. Wird beim
/// Absetzen jedes Spielserver-Requests (send_command) für dessen gesamte
/// Laufzeit gesperrt, da die dafür benötigte SimpleSession Teil davon ist.
struct AppState {
    sessions: Vec<SimpleSession>,
    selected: Option<usize>,
    own_guild: Option<OwnGuild>,
    scan_data: Option<ScanData>,
    /// Scan settings for the next run
    scan_settings: ScanSettings,
    /// Gecachte Kampfwerte (via ViewPlayer) der eigenen aktiven Mitglieder,
    /// um sie nicht vor jeder Simulation neu holen zu müssen. Wird
    /// verworfen, wenn sich das Aktiv-Roster ändert, der Cache zu alt ist,
    /// oder der Charakter gewechselt wird.
    own_fighters_cache: Option<OwnFightersCache>,
    /// Gecachte Kampfwerte gegnerischer Gilden, pro Gildenname. Verhindert,
    /// dass ein wiederholter Simulationslauf für dieselbe(n) Gilde(n) (z.B.
    /// mit anderer Pilz-Anzahl) erneut alle Mitglieder per ViewPlayer holt.
    enemy_fighters_cache: HashMap<String, EnemyFightersCache>,
}

/// Fortschritts-/Abbruch-Zustand für Scan und Simulation, bewusst in einem
/// EIGENEN Lock getrennt von `AppState`. Grund: `AppState` wird für die
/// gesamte Dauer jedes einzelnen Spielserver-Requests gesperrt (die
/// SimpleSession lebt dort) — läge der Fortschritt im selben Lock, würden
/// `/api/progress`, `/api/scan/abort` etc. so lange blockieren, wie der
/// gerade laufende Request zum Spielserver braucht.
#[derive(Default)]
struct ProgressState {
    scan: ScanProgress,
    sim: SimProgress,
    /// Cancellation flag (set via /api/scan/abort)
    cancel_scan: bool,
    /// Cancellation flag (set via /api/simulate/abort)
    cancel_sim: bool,
}

#[derive(Serialize, Clone, Default)]
struct ScanProgress {
    running: bool,
    phase: String,        // "idle", "hof", "details", "done", "error", "aborted"
    current: u32,
    total: u32,
    message: String,
}

#[derive(Serialize, Clone, Default)]
struct SimProgress {
    running: bool,
    phase: String,        // "idle", "own-roster", "guilds", "done", "error", "aborted"
    current: u32,
    total: u32,
    message: String,
}

/// Gecachte, per ViewPlayer geholte Kampfwerte der eigenen aktiven
/// Mitglieder (inkl. wir selbst, kostenlos aus dem GameState).
struct OwnFightersCache {
    fetched_at: chrono::DateTime<chrono::Local>,
    /// Gildenname, für den dieser Cache gilt (Charakterwechsel invalidiert)
    guild_name: String,
    /// Sortierte Namen der aktiven Mitglieder, die in `fighters` stecken —
    /// Vergleichsbasis, um Roster-Änderungen (Zu-/Abgänge, Aktiv-Wechsel)
    /// sofort zu erkennen, unabhängig vom Zeitfenster.
    active_names: Vec<String>,
    fighters: Vec<Fighter>,
}

/// Wie lange ein Cache der eigenen Kampfwerte ohne Roster-Änderung gültig
/// bleibt, bevor er trotzdem aufgefrischt wird (z.B. weil sich jemand neu
/// ausgerüstet/gelevelt hat, ohne dass sich die Aktiv-Liste ändert).
const OWN_FIGHTERS_TTL_HOURS: i64 = 6;

/// Gecachte, per ViewPlayer geholte Kampfwerte einer einzelnen gegnerischen
/// Gilde.
struct EnemyFightersCache {
    fetched_at: chrono::DateTime<chrono::Local>,
    /// Sortierte Mitgliedernamen, für die `fighters` geholt wurden —
    /// Vergleichsbasis, um Roster-Änderungen (z.B. nach einem neuen Scan)
    /// zu erkennen.
    member_names: Vec<String>,
    fighters: Vec<Fighter>,
}

/// Wie lange ein Cache der gegnerischen Kampfwerte ohne Roster-Änderung
/// gültig bleibt.
const ENEMY_FIGHTERS_TTL_HOURS: i64 = 6;

/// Obergrenze für die Anzahl gleichzeitig gecachter gegnerischer Gilden,
/// damit der Cache bei Nutzung über viele Sessions/Tage hinweg nicht
/// unbegrenzt wächst. Beim Überschreiten wird der älteste Eintrag entfernt.
const ENEMY_FIGHTERS_CACHE_MAX_ENTRIES: usize = 100;

#[derive(Serialize, Deserialize, Clone)]
struct ScanSettings {
    /// How many guilds BELOW our rank to include (default: 300)
    down_limit: u32,
    /// Whether to extend scanning ABOVE our rank using the honor rule (default: true)
    honor_up_scan: bool,
    /// Safety cap for extra pages above the top-20 window (default: 10)
    max_extra_up_pages: u32,
    /// Strict mode: level-to-level compare using own active members (<24h)
    strict_mode: bool,
    /// Strict Top-N: compare enemy against our best N active members (N = enemy members)
    strict_topn: bool,
    /// Komplette Ehrenhalle bis Rang 1 scannen, ohne Ehre-Filter und ohne
    /// max_extra_up_pages als Grenze (nur günstige HoF-Seitenaufrufe, kein
    /// ViewGuild pro Gilde — deshalb auch bei hohem eigenen Rang vertretbar).
    /// Überschreibt honor_up_scan, wenn aktiv (default: false).
    #[serde(default)]
    full_up_scan: bool,
}

impl Default for ScanSettings {
    fn default() -> Self {
        Self {
            down_limit: 300,
            honor_up_scan: true,
            max_extra_up_pages: 10,
            strict_mode: true,
            strict_topn: true,
            full_up_scan: false,
        }
    }
}

type SharedState = Arc<Mutex<AppState>>;
type SharedProgress = Arc<Mutex<ProgressState>>;

/// Als Axum-State übergebenes Bündel beider Locks. Handler nehmen sich per
/// `handles.state`/`handles.progress` gezielt nur den Lock, den sie gerade
/// brauchen.
#[derive(Clone)]
struct AppHandles {
    state: SharedState,
    progress: SharedProgress,
}

// ── API Request/Response Types ───────────────────────────────────────────────

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct CharacterInfo {
    index: usize,
    name: String,
    server: String,
}

#[derive(Deserialize)]
struct SelectCharRequest {
    index: usize,
}


#[derive(Deserialize)]
struct ScanRequest {
    #[serde(default)]
    down_limit: Option<u32>,
    #[serde(default)]
    honor_up_scan: Option<bool>,
    #[serde(default)]
    max_extra_up_pages: Option<u32>,
    #[serde(default)]
    strict_mode: Option<bool>,
    #[serde(default)]
    strict_topn: Option<bool>,
    #[serde(default)]
    full_up_scan: Option<bool>,
}


#[derive(Deserialize)]
struct GuildDetailRequest {
    name: String,
}

#[derive(Deserialize)]
struct SimulateRequest {
    /// Namen der zu simulierenden Gilden (müssen im aktuellen scan_data
    /// vorkommen)
    guild_names: Vec<String>,
    /// Anzahl geladener Riesenpilze im Pilzkatapult, 0-3 (wird geclampt)
    #[serde(default)]
    mushrooms: u8,
    /// Anzahl simulierter Kämpfe pro Gilde (optional, default 2500, wird
    /// geclampt)
    #[serde(default)]
    iterations: Option<u32>,
}

#[derive(Deserialize)]
struct FilterRequest {
    /// Max number of members the enemy guild can have (optional)
    max_members: Option<u32>,
    /// Max level the highest-level member in the enemy guild can have (optional)
    max_highest_level: Option<u16>,
    /// Only show guilds that are not currently being attacked
    hide_attacked: Option<bool>,
    /// Only show guilds that pass strict mode (if strict data exists)
    strict_only: Option<bool>,
}

/// Eine Gilde aus der Ehrenhalle-Rohliste (Phase 1), angereichert mit
/// evtl. schon vorhandenen Simulationsergebnissen (falls über die
/// "Vollständige Ehrenhalle"-Ansicht schon mal probesimuliert). Enthält
/// bewusst KEINE Mitglieder-Level, da Phase 1 die nicht liefert — dafür
/// müsste die Gilde erst per ViewGuild/Simulation nachgeladen werden.
#[derive(Serialize)]
struct HofGuildView {
    name: String,
    rank: u32,
    leader: String,
    member_count: u32,
    honor: u32,
    is_attacked: bool,
    attackable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    sim_win_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sim_mushrooms: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sim_iterations: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sim_evaluated_at: Option<String>,
}

#[derive(Serialize)]
struct FilteredResult {
    own_guild: OwnGuild,
    guilds: Vec<DetailedGuild>,
    /// Vollständige Ehrenhalle-Rohliste aus Phase 1 (alle gescannten
    /// Gilden, auch nicht-angreifbare) — nur gefüllt, wenn `full_up_scan`
    /// beim Scan aktiv war, sonst der übliche Rang/Ehre-Bereich.
    hof_guilds: Vec<HofGuildView>,
    /// Total attackable guilds before filtering
    total_attackable: usize,
    /// After filtering
    filtered_count: usize,
    scanned_at: String,
}

/// Serde-Default für Felder, die bei älteren gespeicherten scan_*.json ohne
/// dieses Feld als `true` gelten sollen (Abwärtskompatibilität).
fn default_true() -> bool {
    true
}

#[derive(Serialize)]
struct ApiResponse<T: Serialize> {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn ok_response<T: Serialize>(data: T) -> Json<ApiResponse<T>> {
    Json(ApiResponse {
        success: true,
        data: Some(data),
        error: None,
    })
}

fn err_response<T: Serialize>(msg: &str) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiResponse {
            success: false,
            data: None,
            error: Some(msg.to_string()),
        }),
    )
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Attack Range Logic
// ═══════════════════════════════════════════════════════════════════════════════

/// Determines if a guild is attackable based on rank and honor rules.
///
/// Rule: All guilds below us (higher rank number), plus guilds up to
/// 20 ranks OR 3000 honor above us.
fn is_attackable(own_rank: u32, own_honor: u32, guild_rank: u32, guild_honor: u32) -> bool {
    if guild_rank > own_rank {
        // Guild is below us in ranking → always attackable
        true
    } else if guild_rank == own_rank {
        // Same rank (our own guild) → not attackable
        false
    } else {
        // Guild is above us (lower rank number)
        let rank_diff = own_rank - guild_rank; // how many ranks above us
        let honor_diff = if guild_honor > own_honor {
            guild_honor - own_honor
        } else {
            0
        };
        rank_diff <= 20 || honor_diff <= 3000
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Handlers
// ═══════════════════════════════════════════════════════════════════════════════

/// POST /api/login
async fn login(
    State(handles): State<AppHandles>,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    log::info!("SSO login for: {}", req.username);

    match SimpleSession::login_sf_account(&req.username, &req.password).await {
        Ok(sessions) => {
            let chars: Vec<CharacterInfo> = sessions
                .iter()
                .enumerate()
                .map(|(i, s)| CharacterInfo {
                    index: i,
                    name: s.username().to_string(),
                    server: s.server_url().to_string(),
                })
                .collect();

            let count = chars.len();
            {
                let mut app = handles.state.lock().await;
                app.sessions = sessions;
                app.selected = None;
                app.own_guild = None;
                app.scan_data = None;
                app.own_fighters_cache = None;
                app.enemy_fighters_cache.clear();
            }
            {
                let mut p = handles.progress.lock().await;
                p.scan = ScanProgress::default();
                p.sim = SimProgress::default();
            }

            log::info!("Login OK – {} character(s)", count);
            Ok(ok_response(chars))
        }
        Err(e) => {
            log::error!("Login failed: {:?}", e);
            Err(err_response::<Vec<CharacterInfo>>(&format!(
                "Login fehlgeschlagen: {:?}", e
            )))
        }
    }
}

/// GET /api/characters – Liste der bereits per SSO geladenen Charaktere,
/// ohne erneuten Login. Ermöglicht den Wechsel zu einem anderen Charakter
/// auf demselben Account, ohne sich neu einzuloggen.
async fn list_characters(State(handles): State<AppHandles>) -> impl IntoResponse {
    let app = handles.state.lock().await;

    if app.sessions.is_empty() {
        return Err(err_response::<Vec<CharacterInfo>>("Nicht eingeloggt"));
    }

    let chars: Vec<CharacterInfo> = app
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| CharacterInfo {
            index: i,
            name: s.username().to_string(),
            server: s.server_url().to_string(),
        })
        .collect();

    Ok(ok_response(chars))
}

/// POST /api/select-character
async fn select_character(
    State(handles): State<AppHandles>,
    Json(req): Json<SelectCharRequest>,
) -> impl IntoResponse {
    let mut app = handles.state.lock().await;

    if req.index >= app.sessions.len() {
        return Err(err_response::<OwnGuild>("Ungültiger Charakter-Index"));
    }

    // Beim (Wieder-)Auswählen eines Charakters gehört der Stand des vorher
    // ausgewählten Charakters nicht mehr zu diesem Kontext — sonst würden
    // z.B. alte Scan-Daten fälschlich für die neue Gilde angezeigt, falls
    // für die neue Gilde noch kein eigener Scan gespeichert ist. Der
    // Kampfwerte-Cache ist ebenfalls charakterspezifisch.
    app.selected = Some(req.index);
    app.own_guild = None;
    app.scan_data = None;
    app.own_fighters_cache = None;
    app.enemy_fighters_cache.clear();
    {
        let mut p = handles.progress.lock().await;
        p.scan = ScanProgress::default();
        p.sim = SimProgress::default();
        p.cancel_scan = false;
        p.cancel_sim = false;
    }

    // Borrow session only as long as needed, then release it before touching other app fields.
    let server = app.sessions[req.index].server_url().to_string();

    let gs_res = {
        let session = &mut app.sessions[req.index];
        session.send_command(Command::Update).await
    };

    match gs_res {
        Ok(gs) => {
            if let Some(guild) = &gs.guild {
                let now = chrono::Local::now();
                let mut members: Vec<MemberInfo> = Vec::new();
                let mut active_members: Vec<MemberInfo> = Vec::new();

                for m in &guild.members {
                    let (last_online, offline_days, is_active_24h) = match m.last_online.as_ref() {
                        Some(ts) => {
                            let dur = now.signed_duration_since(ts.clone());
                            let secs = dur.num_seconds().max(0) as f32;
                            let days = secs / 86400.0;
                            let active = dur < chrono::Duration::days(1);
                            (
                                Some(ts.format("%Y-%m-%d %H:%M:%S").to_string()),
                                Some((days * 10.0).round() / 10.0),
                                Some(active),
                            )
                        }
                        None => (None, None, Some(false)),
                    };

                    let info = MemberInfo {
                        name: m.name.clone(),
                        level: m.level,
                        last_online,
                        offline_days,
                        is_active_24h,
                    };

                    if info.is_active_24h.unwrap_or(false) {
                        active_members.push(info.clone());
                    }
                    members.push(info);
                }

                let info = OwnGuild {
                    name: guild.name.clone(),
                    rank: guild.rank,
                    honor: guild.honor,
                    member_count: members.len(),
                    active_member_count: active_members.len(),
                    members,
                    active_members,
                };

                log::info!(
                    "Guild: {} | Rank #{} | Honor {} | {} members",
                    info.name, info.rank, info.honor, info.member_count
                );
                app.own_guild = Some(info.clone());

                // Try to load existing scan data
                // (server url was captured before we mutated app state)
                let server = server.clone();
                if let Ok(data) = load_scan_data(&server, &info.name) {
                    log::info!("Loaded existing scan from {}", data.scanned_at);
                    app.scan_data = Some(data);
                }

                Ok(ok_response(info))
            } else {
                Err(err_response::<OwnGuild>(
                    "Dieser Charakter ist in keiner Gilde!",
                ))
            }
        }
        Err(e) => Err(err_response::<OwnGuild>(&format!("Fehler: {:?}", e))),
    }
}

/// POST /api/scan – Start scan around our rank (runs in background)
async fn start_scan(
    State(handles): State<AppHandles>,
    Json(req): Json<ScanRequest>,
) -> impl IntoResponse {
    {
        let app = handles.state.lock().await;
        if app.selected.is_none() || app.own_guild.is_none() {
            return Err(err_response::<String>("Kein Charakter/Gilde geladen"));
        }
    }
    {
        let p = handles.progress.lock().await;
        if p.scan.running {
            return Err(err_response::<String>("Scan läuft bereits"));
        }
        if p.sim.running {
            return Err(err_response::<String>("Bitte warten, bis die laufende Simulation fertig ist"));
        }
    }
    {
        let mut app = handles.state.lock().await;
        // Apply scan settings (with defaults)
        let mut s = app.scan_settings.clone();
        if let Some(v) = req.down_limit { s.down_limit = v; }
        if let Some(v) = req.honor_up_scan { s.honor_up_scan = v; }
        if let Some(v) = req.max_extra_up_pages { s.max_extra_up_pages = v; }
        if let Some(v) = req.strict_mode { s.strict_mode = v; }
        if let Some(v) = req.strict_topn { s.strict_topn = v; }
        if let Some(v) = req.full_up_scan { s.full_up_scan = v; }
        // clamp to sane values
        s.down_limit = s.down_limit.clamp(0, 50_000);
        s.max_extra_up_pages = s.max_extra_up_pages.clamp(0, 50);
        app.scan_settings = s;
    }
    {
        let mut p = handles.progress.lock().await;
        p.cancel_scan = false;
    }

    // Start scan in background task
    let handles_clone = handles.clone();
    tokio::spawn(async move {
        if let Err(e) = run_scan(handles_clone.clone()).await {
            log::error!("Scan error: {}", e);
            let mut p = handles_clone.progress.lock().await;
            p.scan = ScanProgress {
                running: false,
                phase: "error".into(),
                current: 0,
                total: 0,
                message: format!("Scan fehlgeschlagen: {}", e),
            };
        }
    });

    Ok(ok_response("Scan gestartet".to_string()))
}

/// POST /api/scan/abort – Request cancellation of a running scan
async fn abort_scan(State(handles): State<AppHandles>) -> impl IntoResponse {
    let mut p = handles.progress.lock().await;

    if !p.scan.running {
        return ok_response("Kein laufender Scan".to_string());
    }

    p.cancel_scan = true;
    ok_response("Abbruch angefordert".to_string())
}

/// The actual scan logic, running as a background task
async fn run_scan(handles: AppHandles) -> Result<(), String> {
    const PAGE_SIZE: u32 = 51;

    // Snapshot required state + reset cancel flag
    let (own_guild, server, settings) = {
        let app = handles.state.lock().await;
        let og = app.own_guild.clone().unwrap();
        let idx = app.selected.unwrap();
        let server = app.sessions[idx].server_url().to_string();
        let settings = app.scan_settings.clone();
        (og, server, settings)
    };
    {
        let mut p = handles.progress.lock().await;
        p.cancel_scan = false;
        p.scan = ScanProgress {
            running: true,
            phase: "hof".into(),
            current: 0,
            total: 0,
            message: "Starte HoF-Scan...".into(),
        };
    }

    let own_rank = own_guild.rank;
    let own_honor = own_guild.honor;

    // Precompute own active levels (<24h) for strict mode (ascending)
    let mut own_active_levels: Vec<u16> = own_guild
        .active_members
        .iter()
        .map(|m| m.level)
        .collect();
    own_active_levels.sort_unstable();

    // Rank window (always include below us up to down_limit)
    let down_limit = settings.down_limit.max(0);
    let rank_down_end = own_rank.saturating_add(down_limit);

    // Always include up to 20 ranks above us (rank window)
    let rank_up_start = own_rank.saturating_sub(20).max(1);

    // Pages that cover the mandatory rank window
    let page_low = (rank_up_start.saturating_sub(1)) / PAGE_SIZE;
    let page_high = (rank_down_end.saturating_sub(1)) / PAGE_SIZE;

    log::info!(
        "Phase 1: Scanning HoF window ranks [{}..={}], pages [{}..={}] (down_limit={}, honor_up_scan={}, max_extra_up_pages={})",
        rank_up_start,
        rank_down_end,
        page_low,
        page_high,
        down_limit,
        settings.honor_up_scan,
        settings.max_extra_up_pages
    );

    // Helper: check cancellation quickly
    async fn cancelled(progress: &SharedProgress) -> bool {
        progress.lock().await.cancel_scan
    }

    let mut all_hof_guilds: Vec<HofGuild> = Vec::new();

    // ── 1a) Scan mandatory rank-window pages (includes up to 20 above + down_limit below)
    let total_pages = page_high.saturating_sub(page_low) + 1;
    {
        let mut p = handles.progress.lock().await;
        p.scan.total = total_pages;
        p.scan.message = format!(
            "Scanne Rangbereich #{}..#{} ({} Seiten)...",
            rank_up_start,
            rank_down_end,
            total_pages
        );
    }

    for (i, page) in (page_low..=page_high).enumerate() {
        if cancelled(&handles.progress).await {
            log::info!("HoF scan cancelled during window pages");
            break;
        }

        {
            let mut p = handles.progress.lock().await;
            p.scan.current = (i as u32) + 1;
            p.scan.message = format!(
                "HoF Seite {} wird gescannt... ({} Gilden gesammelt)",
                page + 1,
                all_hof_guilds.len()
            );
        }

        let guilds_on_page = {
            let mut app = handles.state.lock().await;
            let idx = app.selected.unwrap();
            let session = &mut app.sessions[idx];
            match session.send_command(Command::HallOfFameGroupPage { page }).await {
                Ok(gs) => gs.hall_of_fames.guilds.clone(),
                Err(e) => {
                    log::warn!("HoF page {} error: {:?}", page, e);
                    Vec::new()
                }
            }
        };

        for hg in &guilds_on_page {
            if hg.name == own_guild.name {
                continue;
            }

            // Keep only what we need:
            // - below us up to down_limit
            // - above us up to 20 ranks
            let in_rank_window = (hg.rank >= rank_up_start && hg.rank < own_rank)
                || (hg.rank > own_rank && hg.rank <= rank_down_end);

            if !in_rank_window {
                continue;
            }

            let attackable = is_attackable(own_rank, own_honor, hg.rank, hg.honor);

            all_hof_guilds.push(HofGuild {
                name: hg.name.clone(),
                rank: hg.rank,
                leader: hg.leader.clone(),
                member_count: hg.member_count,
                honor: hg.honor,
                is_attacked: hg.is_attacked,
                attackable,
            });
        }

        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // ── 1b) Honor-Up-Scan / komplette Ehrenhalle bis Rang 1 ────────────────
    // full_up_scan: keine Ehre-Filterung, max_extra_up_pages wird ignoriert
    // — geht immer bis Seite 0 (Rang 1) durch. Nur günstige HoF-
    // Seitenaufrufe, kein ViewGuild pro Gilde, deshalb auch bei hohem
    // eigenen Rang vertretbar.
    // honor_up_scan (ohne full_up_scan): wie bisher ehre-gefiltert, aber
    // OHNE verfrühten Abbruch bei der ersten leeren Seite — sonst könnte
    // eine "Lücke" in der Ehre-Kette angreifbare Gilden dahinter
    // verstecken. max_extra_up_pages ist jetzt die alleinige, ehrliche
    // Grenze.
    if (settings.full_up_scan || settings.honor_up_scan)
        && page_low > 0
        && !cancelled(&handles.progress).await
    {
        let mut extra_scanned = 0u32;
        let mut page = page_low - 1;

        loop {
            if cancelled(&handles.progress).await {
                log::info!("HoF scan cancelled during honor-up pages");
                break;
            }
            if !settings.full_up_scan && extra_scanned >= settings.max_extra_up_pages {
                break;
            }

            {
                let mut p = handles.progress.lock().await;
                p.scan.message = if settings.full_up_scan {
                    format!("Vollständige Ehrenhalle: Seite {} wird geladen...", page + 1)
                } else {
                    format!("HoF (Ehre-Regel): Seite {} wird geprüft...", page + 1)
                };
            }

            let guilds_on_page = {
            let mut app = handles.state.lock().await;
            let idx = app.selected.unwrap();
            let session = &mut app.sessions[idx];
            match session.send_command(Command::HallOfFameGroupPage { page }).await {
                Ok(gs) => gs.hall_of_fames.guilds.clone(),
                Err(e) => {
                    log::warn!("HoF page {} error: {:?}", page, e);
                    Vec::new()
                }
            }
        };

            for hg in &guilds_on_page {
                if hg.name == own_guild.name {
                    continue;
                }

                // Only above us
                if hg.rank >= own_rank {
                    continue;
                }

                // Honor-Regel nur anwenden, wenn wir nicht die komplette
                // Ehrenhalle unabhängig davon sehen wollen.
                if !settings.full_up_scan && hg.honor > own_honor.saturating_add(3000) {
                    continue;
                }

                let attackable = is_attackable(own_rank, own_honor, hg.rank, hg.honor);

                // Avoid duplicates (same guild can appear across pages? usually no, but safe)
                if all_hof_guilds.iter().any(|g| g.name == hg.name) {
                    continue;
                }

                all_hof_guilds.push(HofGuild {
                    name: hg.name.clone(),
                    rank: hg.rank,
                    leader: hg.leader.clone(),
                    member_count: hg.member_count,
                    honor: hg.honor,
                    is_attacked: hg.is_attacked,
                    attackable,
                });
            }

            extra_scanned += 1;

            if page == 0 {
                break;
            }
            page -= 1;
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    // Sort by rank (nice for UI/export)
    all_hof_guilds.sort_by_key(|g| g.rank);

    let attackable_names: Vec<(String, u32, u32, bool)> = all_hof_guilds
        .iter()
        .filter(|g| g.attackable)
        .map(|g| (g.name.clone(), g.rank, g.honor, g.is_attacked))
        .collect();

    let attackable_count = attackable_names.len();
    log::info!(
        "Phase 1 complete: {} guilds in window, {} attackable",
        all_hof_guilds.len(),
        attackable_count
    );

    // ── Phase 2: Load details for attackable guilds ───────────────────────
    {
        let mut p = handles.progress.lock().await;
        p.scan.phase = "details".into();
        p.scan.current = 0;
        p.scan.total = attackable_count as u32;
        p.scan.message = format!(
            "Phase 2: Lade Details für {} angreifbare Gilden...",
            attackable_count
        );
    }

    let mut detailed_guilds: Vec<DetailedGuild> = Vec::new();

    for (i, (name, rank, honor, is_attacked)) in attackable_names.iter().enumerate() {
        if cancelled(&handles.progress).await {
            log::info!("Details loading cancelled");
            break;
        }

        {
            let mut p = handles.progress.lock().await;
            p.scan.current = i as u32 + 1;
            p.scan.message = format!(
                "Lade Gilde {}/{}: {}",
                i + 1,
                attackable_count,
                name
            );
        }

        let detail = {
            let mut app = handles.state.lock().await;
            let idx = app.selected.unwrap();
            let session = &mut app.sessions[idx];

            match session
                .send_command(Command::ViewGuild {
                    guild_ident: name.clone(),
                })
                .await
            {
                Ok(gs) => {
                    if let Some(other) = gs.lookup.guilds.get(name) {
                        let members: Vec<MemberInfo> = other
                            .members
                            .iter()
                            .map(|m| MemberInfo {
                                name: m.name.clone(),
                                level: m.level,
                                last_online: None,
                                offline_days: None,
                                is_active_24h: None,
                            })
                            .collect();

                        let max_level = members.iter().map(|m| m.level).max().unwrap_or(0);
                        let min_level = members.iter().map(|m| m.level).min().unwrap_or(0);
                        let total_level: u32 = members.iter().map(|m| m.level as u32).sum();

                        // ── Strict mode evaluation: level-to-level compare (ascending) ──
                        let mut strict_evaluated = false;
                        let mut strict_beatable = false;
                        let strict_own_active_members = own_active_levels.len();
                        let mut strict_topn = false;
                        let mut strict_topn_n: usize = 0;
                        let mut strict_fail_index: Option<usize> = None;
                        let mut strict_fail_enemy_level: Option<u16> = None;
                        let mut strict_fail_own_level: Option<u16> = None;
                        let mut strict_fail_reason: Option<String> = None;

                        if settings.strict_mode {
                            strict_evaluated = true;

                            strict_topn = settings.strict_topn;

                            let mut enemy_levels: Vec<u16> = members.iter().map(|m| m.level).collect();
                            enemy_levels.sort_unstable();

                            strict_topn_n = enemy_levels.len();

                            if enemy_levels.len() > strict_own_active_members {
                                strict_beatable = false;
                                strict_fail_reason = Some(format!(
                                    "Zu viele Mitglieder: Gegner {} > eigene aktiv {}",
                                    enemy_levels.len(),
                                    strict_own_active_members
                                ));
                            } else if strict_own_active_members == 0 {
                                strict_beatable = false;
                                strict_fail_reason = Some("Keine aktiven eigenen Mitglieder (>= 1 Tag offline wird ausgeklammert).".to_string());
                            } else {
                                if settings.strict_topn {
                                    // Top-N compare: enemy against our best N active members (N = enemy members)
                                    let n = enemy_levels.len();
                                    let start = strict_own_active_members.saturating_sub(n);
                                    let own_slice: &[u16] = &own_active_levels[start..];

                                    strict_beatable = true;
                                    for i in 0..n {
                                        if enemy_levels[i] > own_slice[i] {
                                            strict_beatable = false;
                                            strict_fail_index = Some(i);
                                            strict_fail_enemy_level = Some(enemy_levels[i]);
                                            strict_fail_own_level = Some(own_slice[i]);
                                            strict_fail_reason = Some(format!(
                                                "Slot {}: Gegner {} > eigene {} (Top-N, aufsteigend sortiert)",
                                                i + 1,
                                                enemy_levels[i],
                                                own_slice[i]
                                            ));
                                            break;
                                        }
                                    }
                                } else {
                                    // Full roster compare: enemy against our lowest active members (ascending)
                                    strict_beatable = true;
                                    for i in 0..enemy_levels.len() {
                                        if enemy_levels[i] > own_active_levels[i] {
                                            strict_beatable = false;
                                            strict_fail_index = Some(i);
                                            strict_fail_enemy_level = Some(enemy_levels[i]);
                                            strict_fail_own_level = Some(own_active_levels[i]);
                                            strict_fail_reason = Some(format!(
                                                "Slot {}: Gegner {} > eigene {} (aufsteigend sortiert)",
                                                i + 1,
                                                enemy_levels[i],
                                                own_active_levels[i]
                                            ));
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        Some(DetailedGuild {
                            name: name.clone(),
                            rank: *rank,
                            honor: *honor,
                            member_count: members.len(),
                            members,
                            max_level,
                            min_level,
                            total_level,
                            finished_raids: other.finished_raids,
                            is_attacked: *is_attacked,
                            // attackable_names enthält per Konstruktion nur
                            // Gilden, die is_attackable() bereits bestanden
                            // haben (siehe Filter weiter oben).
                            attackable: true,
                            strict_evaluated,
                            strict_beatable,
                            strict_own_active_members,
                            strict_topn,
                            strict_topn_n,
                            strict_fail_index,
                            strict_fail_enemy_level,
                            strict_fail_own_level,
                            strict_fail_reason,
                            sim_win_ratio: None,
                            sim_mushrooms: None,
                            sim_iterations: None,
                            sim_evaluated_at: None,
                        })
                    } else {
                        log::warn!("Guild {} not in lookup after ViewGuild", name);
                        None
                    }
                }
                Err(e) => {
                    log::warn!("ViewGuild {} failed: {:?}", name, e);
                    None
                }
            }
        };

        if let Some(d) = detail {
            detailed_guilds.push(d);
        }

        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    // ── Save results (also on abort) ─────────────────────────────────────
    let scan_data = ScanData {
        scanned_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        server: server.clone(),
        own_guild: own_guild.clone(),
        hof_guilds: all_hof_guilds,
        detailed_guilds,
    };

    if let Err(e) = save_scan_data(&scan_data) {
        log::error!("Failed to save scan data: {}", e);
    }

    let was_cancelled = cancelled(&handles.progress).await;

    {
        let mut app = handles.state.lock().await;
        app.scan_data = Some(scan_data);
    }
    {
        let mut p = handles.progress.lock().await;
        p.scan = ScanProgress {
            running: false,
            phase: if was_cancelled { "aborted".into() } else { "done".into() },
            current: p.scan.current,
            total: p.scan.total,
            message: if was_cancelled {
                "Scan abgebrochen – Teilergebnis gespeichert.".into()
            } else {
                "Scan abgeschlossen.".into()
            },
        };
        p.cancel_scan = false;
    }

    Ok(())
}

/// GET /api/progress – Poll scan progress
async fn get_progress(State(handles): State<AppHandles>) -> impl IntoResponse {
    let p = handles.progress.lock().await;
    ok_response(p.scan.clone())
}

/// POST /api/results – Get filtered results
async fn get_results(
    State(handles): State<AppHandles>,
    Json(filter): Json<FilterRequest>,
) -> impl IntoResponse {
    let app = handles.state.lock().await;

    let scan = match &app.scan_data {
        Some(s) => s,
        None => return Err(err_response::<FilteredResult>("Keine Scan-Daten vorhanden")),
    };

    let total_attackable = scan.detailed_guilds.iter().filter(|g| g.attackable).count();
    let strict_available = scan
        .detailed_guilds
        .iter()
        .any(|g| g.attackable && g.strict_evaluated);
    // "winnable_only" default: if strict data exists, default to strict-only.
    let strict_only = filter.strict_only.unwrap_or(true);

    let filtered: Vec<DetailedGuild> = scan
        .detailed_guilds
        .iter()
        .filter(|g| {
            // Nur echte Angriffs-Kandidaten in der normalen Ergebnistabelle
            // — detailed_guilds kann inzwischen auch nicht-angreifbare
            // Gilden enthalten, die nur zum Vormerken/Probesimulieren aus
            // der vollständigen Ehrenhalle-Ansicht heraus geladen wurden.
            if !g.attackable {
                return false;
            }
            // Filter: max members
            if let Some(max_m) = filter.max_members {
                if g.member_count as u32 > max_m {
                    return false;
                }
            }
            // Filter: max level of highest member
            if let Some(max_lvl) = filter.max_highest_level {
                if g.max_level > max_lvl {
                    return false;
                }
            }
            // Filter: hide attacked
            if filter.hide_attacked.unwrap_or(false) && g.is_attacked {
                return false;
            }
            // Filter: strict only (default ON if strict data exists)
            if strict_only && strict_available {
                if !g.strict_evaluated || !g.strict_beatable {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect();

    let filtered_count = filtered.len();

    // Vollständige Ehrenhalle-Rohliste, angereichert mit evtl. schon
    // vorhandenen Simulationsergebnissen aus detailed_guilds (per Name
    // nachgeschlagen, da eine probesimulierte, noch nicht angreifbare
    // Gilde dort inzwischen ein Eintrag sein kann).
    let hof_guilds: Vec<HofGuildView> = scan
        .hof_guilds
        .iter()
        .map(|hg| {
            let sim = scan.detailed_guilds.iter().find(|d| d.name == hg.name);
            HofGuildView {
                name: hg.name.clone(),
                rank: hg.rank,
                leader: hg.leader.clone(),
                member_count: hg.member_count,
                honor: hg.honor,
                is_attacked: hg.is_attacked,
                attackable: hg.attackable,
                sim_win_ratio: sim.and_then(|d| d.sim_win_ratio),
                sim_mushrooms: sim.and_then(|d| d.sim_mushrooms),
                sim_iterations: sim.and_then(|d| d.sim_iterations),
                sim_evaluated_at: sim.and_then(|d| d.sim_evaluated_at.clone()),
            }
        })
        .collect();

    Ok(ok_response(FilteredResult {
        own_guild: scan.own_guild.clone(),
        guilds: filtered,
        hof_guilds,
        total_attackable,
        filtered_count,
        scanned_at: scan.scanned_at.clone(),
    }))
}

/// POST /api/guild-details – View details of a specific guild (live request)
async fn guild_details(
    State(handles): State<AppHandles>,
    Json(req): Json<GuildDetailRequest>,
) -> impl IntoResponse {
    // First check if we already have it in scan data
    {
        let app = handles.state.lock().await;
        if let Some(scan) = &app.scan_data {
            if let Some(guild) = scan.detailed_guilds.iter().find(|g| g.name == req.name) {
                return Ok(ok_response(guild.clone()));
            }
        }
    }

    // Otherwise fetch live
    let mut app = handles.state.lock().await;
    let idx = match app.selected {
        Some(i) => i,
        None => return Err(err_response::<DetailedGuild>("Kein Charakter ausgewählt")),
    };
    // Für die attackable-Berechnung gebraucht (own_rank/own_honor), bevor
    // session gleich mutabel geliehen wird.
    let own_rank_honor = app.own_guild.as_ref().map(|g| (g.rank, g.honor));

    let session = &mut app.sessions[idx];
    match session.send_command(Command::ViewGuild {
        guild_ident: req.name.clone(),
    }).await {
        Ok(gs) => {
            if let Some(other) = gs.lookup.guilds.get(&req.name) {
                let members: Vec<MemberInfo> = other
                    .members
                    .iter()
                    .map(|m| MemberInfo { name: m.name.clone(), level: m.level, last_online: None, offline_days: None, is_active_24h: None })
                    .collect();

                let max_level = members.iter().map(|m| m.level).max().unwrap_or(0);
                let min_level = members.iter().map(|m| m.level).min().unwrap_or(0);
                let total_level: u32 = members.iter().map(|m| m.level as u32).sum();
                let attackable = own_rank_honor
                    .map(|(own_rank, own_honor)| {
                        is_attackable(own_rank, own_honor, other.rank as u32, other.honor)
                    })
                    .unwrap_or(false);

                Ok(ok_response(DetailedGuild {
                    name: req.name,
                    rank: other.rank as u32,
                    honor: other.honor,
                    member_count: members.len(),
                    members,
                    max_level,
                    min_level,
                    total_level,
                    finished_raids: other.finished_raids,
                    is_attacked: false,
                    attackable,
                    strict_evaluated: false,
                    strict_beatable: false,
                    strict_own_active_members: 0,
                    strict_topn: false,
                    strict_topn_n: 0,
                    strict_fail_index: None,
                    strict_fail_enemy_level: None,
                    strict_fail_own_level: None,
                    strict_fail_reason: None,
                    sim_win_ratio: None,
                    sim_mushrooms: None,
                    sim_iterations: None,
                    sim_evaluated_at: None,
                }))
            } else {
                Err(err_response::<DetailedGuild>("Gilde nicht gefunden"))
            }
        }
        Err(e) => Err(err_response::<DetailedGuild>(&format!("Fehler: {:?}", e))),
    }
}

/// GET /api/status
async fn status(State(handles): State<AppHandles>) -> impl IntoResponse {
    let app = handles.state.lock().await;

    #[derive(Serialize)]
    struct StatusInfo {
        logged_in: bool,
        character_selected: bool,
        guild_loaded: bool,
        guild_name: Option<String>,
        has_scan_data: bool,
        scan_date: Option<String>,
    }

    ok_response(StatusInfo {
        logged_in: !app.sessions.is_empty(),
        character_selected: app.selected.is_some(),
        guild_loaded: app.own_guild.is_some(),
        guild_name: app.own_guild.as_ref().map(|g| g.name.clone()),
        has_scan_data: app.scan_data.is_some(),
        scan_date: app.scan_data.as_ref().map(|s| s.scanned_at.clone()),
    })
}

/// POST /api/logout
async fn logout(State(handles): State<AppHandles>) -> impl IntoResponse {
    {
        let mut app = handles.state.lock().await;
        app.sessions.clear();
        app.selected = None;
        app.own_guild = None;
        app.scan_data = None;
        app.own_fighters_cache = None;
        app.enemy_fighters_cache.clear();
    }
    {
        let mut p = handles.progress.lock().await;
        p.scan = ScanProgress::default();
        p.sim = SimProgress::default();
    }
    ok_response("Ausgeloggt")
}

/// GET /api/export – Export scan data as JSON download
async fn export_data(State(handles): State<AppHandles>) -> impl IntoResponse {
    let app = handles.state.lock().await;
    match &app.scan_data {
        Some(data) => {
            let json = serde_json::to_string_pretty(data).unwrap_or_default();
            Ok((
                StatusCode::OK,
                [
                    ("Content-Type", "application/json"),
                    ("Content-Disposition", "attachment; filename=\"scan_data.json\""),
                ],
                json,
            ))
        }
        None => Err((
            StatusCode::NOT_FOUND,
            [("Content-Type", "text/plain"), ("Content-Disposition", "inline")],
            "Keine Scan-Daten vorhanden".to_string(),
        )),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Battle Simulation
// ═══════════════════════════════════════════════════════════════════════════════

/// Pause zwischen ViewPlayer-Calls beim Aufbau von Kampfwerten. Analog zu
/// sfguildsv2/rust_examples/character_sync.rs (dort 700ms, mit Hinweis auf
/// eine ~2-Minuten-Session-Grenze bei zu vielen ViewPlayer-Calls).
const SIM_VIEWPLAYER_DELAY_MS: u64 = 700;

/// Wendet das Pilzkatapult auf eine Gegner-Aufstellung an: pro geladenem
/// Riesenpilz wird ein zufälliges Ziel gezogen (mit Zurücklegen — derselbe
/// Gegner kann mehrfach getroffen werden) und ihm 50 Prozentpunkte seines
/// ursprünglichen max_health additiv abgezogen (2 Treffer aufs selbe Ziel =
/// 0% Leben übrig, nicht 25% wie bei multiplikativem Stacking). Gibt eine
/// neue, unabhängige Kopie zurück — `base` bleibt unverändert, damit jede
/// Simulations-Iteration mit frisch gewürfelten Zielen startet (echte
/// Gildenkämpfe würfeln die Pilz-Ziele pro Kampf neu).
fn apply_mushrooms(base: &[Fighter], mushrooms: u8) -> Vec<Fighter> {
    let mut right = base.to_vec();
    if mushrooms == 0 || right.is_empty() {
        return right;
    }
    let mut hits = vec![0u32; right.len()];
    for _ in 0..mushrooms {
        let target = fastrand::usize(0..right.len());
        hits[target] += 1;
    }
    for (fighter, hit_count) in right.iter_mut().zip(hits.iter()) {
        if *hit_count > 0 {
            let remaining = (1.0 - 0.5 * f64::from(*hit_count)).max(0.0);
            fighter.max_health *= remaining;
        }
    }
    right
}

/// Holt die Kampfwerte der eigenen aktiven Mitglieder — aus dem Cache, wenn
/// er noch zum aktuellen Aktiv-Roster passt und nicht älter als
/// `OWN_FIGHTERS_TTL_HOURS` ist, sonst frisch per ViewPlayer (+ wir selbst
/// kostenlos aus dem GameState). Aktualisiert den Cache nach einem Refresh.
async fn get_own_fighters(state: &SharedState) -> Result<Vec<Fighter>, String> {
    let (idx, own_name, guild_name, mut active_names) = {
        let app = state.lock().await;
        let idx = app.selected.ok_or_else(|| "Kein Charakter ausgewählt".to_string())?;
        let own_name = app.sessions[idx].username().to_string();
        let og = app.own_guild.as_ref().ok_or_else(|| "Keine Gilde geladen".to_string())?;
        let names: Vec<String> = og.active_members.iter().map(|m| m.name.clone()).collect();
        (idx, own_name, og.name.clone(), names)
    };
    active_names.sort();

    {
        let app = state.lock().await;
        if let Some(cache) = &app.own_fighters_cache {
            let fresh_enough = chrono::Local::now().signed_duration_since(cache.fetched_at)
                < chrono::Duration::hours(OWN_FIGHTERS_TTL_HOURS);
            if fresh_enough && cache.guild_name == guild_name && cache.active_names == active_names {
                log::info!("Own-fighters cache hit ({} Kämpfer, vom {})", cache.fighters.len(), cache.fetched_at);
                return Ok(cache.fighters.clone());
            }
        }
    }

    log::info!("Own-fighters cache miss/stale, hole {} aktive Mitglieder frisch", active_names.len());

    // Wir selbst: frisches Update, kostenlos aus dem GameState (kein ViewPlayer nötig).
    let mut fighters: Vec<Fighter> = Vec::new();
    {
        let mut app = state.lock().await;
        let session = &mut app.sessions[idx];
        let gs = session
            .send_command(Command::Update)
            .await
            .map_err(|e| format!("Update fehlgeschlagen: {:?}", e))?;
        let squad = PlayerFighterSquad::new(gs);
        fighters.push(Fighter::from(&squad.character));
    }

    // Alle anderen aktiven Mitglieder per ViewPlayer.
    let mut calls_made = 0u32;
    for name in active_names.iter().filter(|n| n.as_str() != own_name) {
        if calls_made > 0 {
            tokio::time::sleep(Duration::from_millis(SIM_VIEWPLAYER_DELAY_MS)).await;
        }
        calls_made += 1;

        let uf = {
            let mut app = state.lock().await;
            let session = &mut app.sessions[idx];
            match session.send_command(Command::ViewPlayer { ident: name.clone() }).await {
                Ok(gs) => gs.lookup.lookup_name(name).map(UpgradeableFighter::from_other),
                Err(e) => {
                    log::warn!("ViewPlayer für eigenes Mitglied '{}' fehlgeschlagen: {:?}", name, e);
                    None
                }
            }
        };

        match uf {
            Some(uf) => fighters.push(Fighter::from(&uf)),
            None => log::warn!("Kein ViewPlayer-Ergebnis für eigenes Mitglied '{}'", name),
        }
    }

    {
        let mut app = state.lock().await;
        app.own_fighters_cache = Some(OwnFightersCache {
            fetched_at: chrono::Local::now(),
            guild_name,
            active_names,
            fighters: fighters.clone(),
        });
    }

    Ok(fighters)
}

/// Stellt sicher, dass eine Gilde als `DetailedGuild`-Eintrag in den
/// Scan-Daten existiert. Gilden aus der "Vollständigen Ehrenhalle"-Ansicht
/// sind (noch) nicht angreifbar und wurden deshalb nie über Phase 2 des
/// Scans geladen — hier werden sie bei Bedarf live per ViewGuild
/// nachgeholt, ohne Strict-Mode-Auswertung (die bräuchte den
/// own_active_levels-Schnappschuss eines laufenden Scans, den es außerhalb
/// eines Scans nicht gibt).
async fn ensure_detailed_guild(state: &SharedState, guild_name: &str) -> Result<(), String> {
    {
        let app = state.lock().await;
        let scan = app.scan_data.as_ref().ok_or_else(|| "Keine Scan-Daten vorhanden".to_string())?;
        if scan.detailed_guilds.iter().any(|g| g.name == guild_name) {
            return Ok(());
        }
    }

    log::info!("Gilde '{}' noch nicht in Scan-Daten, hole live per ViewGuild nach", guild_name);

    let (idx, own_rank_honor) = {
        let app = state.lock().await;
        let idx = app.selected.ok_or_else(|| "Kein Charakter ausgewählt".to_string())?;
        let own_rank_honor = app.own_guild.as_ref().map(|g| (g.rank, g.honor));
        (idx, own_rank_honor)
    };

    let mut app = state.lock().await;
    let session = &mut app.sessions[idx];
    let gs = session
        .send_command(Command::ViewGuild { guild_ident: guild_name.to_string() })
        .await
        .map_err(|e| format!("ViewGuild für '{}' fehlgeschlagen: {:?}", guild_name, e))?;

    let other = gs
        .lookup
        .guilds
        .get(guild_name)
        .ok_or_else(|| format!("Gilde '{}' nicht gefunden", guild_name))?;

    let members: Vec<MemberInfo> = other
        .members
        .iter()
        .map(|m| MemberInfo {
            name: m.name.clone(),
            level: m.level,
            last_online: None,
            offline_days: None,
            is_active_24h: None,
        })
        .collect();
    let max_level = members.iter().map(|m| m.level).max().unwrap_or(0);
    let min_level = members.iter().map(|m| m.level).min().unwrap_or(0);
    let total_level: u32 = members.iter().map(|m| m.level as u32).sum();
    let attackable = own_rank_honor
        .map(|(own_rank, own_honor)| is_attackable(own_rank, own_honor, other.rank as u32, other.honor))
        .unwrap_or(false);

    let detailed = DetailedGuild {
        name: guild_name.to_string(),
        rank: other.rank as u32,
        honor: other.honor,
        member_count: members.len(),
        members,
        max_level,
        min_level,
        total_level,
        finished_raids: other.finished_raids,
        is_attacked: false,
        attackable,
        strict_evaluated: false,
        strict_beatable: false,
        strict_own_active_members: 0,
        strict_topn: false,
        strict_topn_n: 0,
        strict_fail_index: None,
        strict_fail_enemy_level: None,
        strict_fail_own_level: None,
        strict_fail_reason: None,
        sim_win_ratio: None,
        sim_mushrooms: None,
        sim_iterations: None,
        sim_evaluated_at: None,
    };

    if let Some(scan) = app.scan_data.as_mut() {
        scan.detailed_guilds.push(detailed);
    }

    Ok(())
}

/// Holt die Kampfwerte einer gegnerischen Gilde — aus dem Cache, wenn er
/// noch zum aktuellen Mitglieder-Roster laut Scan-Daten passt und nicht
/// älter als `ENEMY_FIGHTERS_TTL_HOURS` ist, sonst frisch per ViewPlayer.
/// Verhindert, dass ein wiederholter Simulationslauf für dieselbe Gilde
/// (z.B. mit anderer Pilz-Anzahl) erneut alle Mitglieder abfragt.
async fn get_enemy_fighters(state: &SharedState, guild_name: &str) -> Result<Vec<Fighter>, String> {
    ensure_detailed_guild(state, guild_name).await?;

    let (idx, mut member_names) = {
        let app = state.lock().await;
        let idx = app.selected.ok_or_else(|| "Kein Charakter ausgewählt".to_string())?;
        let scan = app.scan_data.as_ref().ok_or_else(|| "Keine Scan-Daten vorhanden".to_string())?;
        let guild = scan
            .detailed_guilds
            .iter()
            .find(|g| g.name == guild_name)
            .ok_or_else(|| format!("Gilde '{}' nicht in Scan-Daten gefunden", guild_name))?;
        let names: Vec<String> = guild.members.iter().map(|m| m.name.clone()).collect();
        (idx, names)
    };
    member_names.sort();

    {
        let app = state.lock().await;
        if let Some(cache) = app.enemy_fighters_cache.get(guild_name) {
            let fresh_enough = chrono::Local::now().signed_duration_since(cache.fetched_at)
                < chrono::Duration::hours(ENEMY_FIGHTERS_TTL_HOURS);
            if fresh_enough && cache.member_names == member_names {
                log::info!(
                    "Enemy-fighters cache hit für '{}' ({} Kämpfer, vom {})",
                    guild_name, cache.fighters.len(), cache.fetched_at
                );
                return Ok(cache.fighters.clone());
            }
        }
    }

    log::info!(
        "Enemy-fighters cache miss/stale für '{}', hole {} Mitglieder frisch",
        guild_name, member_names.len()
    );

    let mut fighters: Vec<Fighter> = Vec::new();
    let mut calls_made = 0u32;
    for name in &member_names {
        if calls_made > 0 {
            tokio::time::sleep(Duration::from_millis(SIM_VIEWPLAYER_DELAY_MS)).await;
        }
        calls_made += 1;

        let uf = {
            let mut app = state.lock().await;
            let session = &mut app.sessions[idx];
            match session.send_command(Command::ViewPlayer { ident: name.clone() }).await {
                Ok(gs) => gs.lookup.lookup_name(name).map(UpgradeableFighter::from_other),
                Err(e) => {
                    log::warn!("ViewPlayer für Gegner '{}' fehlgeschlagen: {:?}", name, e);
                    None
                }
            }
        };

        if let Some(uf) = uf {
            fighters.push(Fighter::from(&uf));
        }
    }

    {
        let mut app = state.lock().await;
        // Cache-Obergrenze: ältesten Eintrag entfernen, falls voll und
        // diese Gilde noch nicht drin ist.
        if app.enemy_fighters_cache.len() >= ENEMY_FIGHTERS_CACHE_MAX_ENTRIES
            && !app.enemy_fighters_cache.contains_key(guild_name)
        {
            if let Some(oldest_key) = app
                .enemy_fighters_cache
                .iter()
                .min_by_key(|(_, c)| c.fetched_at)
                .map(|(k, _)| k.clone())
            {
                app.enemy_fighters_cache.remove(&oldest_key);
            }
        }
        app.enemy_fighters_cache.insert(
            guild_name.to_string(),
            EnemyFightersCache {
                fetched_at: chrono::Local::now(),
                member_names,
                fighters: fighters.clone(),
            },
        );
    }

    Ok(fighters)
}

/// Simuliert den Kampf gegen eine einzelne Gilde.
async fn simulate_one_guild(
    state: &SharedState,
    own_fighters: &[Fighter],
    guild_name: &str,
    mushrooms: u8,
    iterations: u32,
) -> Result<f64, String> {
    let enemy_fighters = get_enemy_fighters(state, guild_name).await?;

    if own_fighters.is_empty() || enemy_fighters.is_empty() {
        return Err("Keine Kämpfer auf einer Seite, Simulation nicht möglich".to_string());
    }

    // Pilz-Ziele werden PRO Einzelkampf neu gewürfelt (siehe apply_mushrooms),
    // deshalb manueller Loop mit iterations=1 statt einem simulate_battle-
    // Aufruf mit iterations=N.
    let mut won_fights = 0u32;
    for _ in 0..iterations {
        let right = apply_mushrooms(&enemy_fighters, mushrooms);
        let result = simulate_battle(own_fighters, &right, 1, false);
        won_fights += result.won_fights;
    }

    Ok(f64::from(won_fights) / f64::from(iterations))
}

/// POST /api/simulate – Start battle simulation for selected guilds (runs in background)
async fn start_simulate(
    State(handles): State<AppHandles>,
    Json(req): Json<SimulateRequest>,
) -> impl IntoResponse {
    if req.guild_names.is_empty() {
        return Err(err_response::<String>("Keine Gilden ausgewählt"));
    }
    {
        let app = handles.state.lock().await;
        if app.selected.is_none() || app.scan_data.is_none() {
            return Err(err_response::<String>("Kein Charakter/Scan geladen"));
        }
    }
    {
        let p = handles.progress.lock().await;
        if p.sim.running {
            return Err(err_response::<String>("Simulation läuft bereits"));
        }
        if p.scan.running {
            return Err(err_response::<String>("Bitte warten, bis der laufende Scan fertig ist"));
        }
    }
    {
        let mut p = handles.progress.lock().await;
        p.cancel_sim = false;
    }

    let mushrooms = req.mushrooms.min(3);
    let iterations = req.iterations.unwrap_or(2500).clamp(100, 20_000);
    let guild_names = req.guild_names.clone();

    let handles_clone = handles.clone();
    tokio::spawn(async move {
        run_simulate(handles_clone, guild_names, mushrooms, iterations).await;
    });

    Ok(ok_response("Simulation gestartet".to_string()))
}

/// The actual simulation logic, running as a background task
async fn run_simulate(handles: AppHandles, guild_names: Vec<String>, mushrooms: u8, iterations: u32) {
    {
        let mut p = handles.progress.lock().await;
        p.sim = SimProgress {
            running: true,
            phase: "own-roster".into(),
            current: 0,
            total: guild_names.len() as u32,
            message: "Aktualisiere eigene Kampfwerte...".into(),
        };
    }

    let own_fighters = match get_own_fighters(&handles.state).await {
        Ok(f) => f,
        Err(e) => {
            let mut p = handles.progress.lock().await;
            p.sim = SimProgress {
                running: false,
                phase: "error".into(),
                current: 0,
                total: 0,
                message: format!("Fehler beim Laden eigener Kampfwerte: {}", e),
            };
            return;
        }
    };

    {
        let mut p = handles.progress.lock().await;
        p.sim.phase = "guilds".into();
        p.sim.message = "Simuliere Gilden...".into();
    }

    let mut results: Vec<(String, f64)> = Vec::new();

    for (i, name) in guild_names.iter().enumerate() {
        if handles.progress.lock().await.cancel_sim {
            log::info!("Simulation cancelled");
            break;
        }
        {
            let mut p = handles.progress.lock().await;
            p.sim.current = i as u32 + 1;
            p.sim.message = format!("Simuliere {}/{}: {}", i + 1, guild_names.len(), name);
        }

        match simulate_one_guild(&handles.state, &own_fighters, name, mushrooms, iterations).await {
            Ok(ratio) => results.push((name.clone(), ratio)),
            Err(e) => log::warn!("Simulation für '{}' fehlgeschlagen: {}", name, e),
        }
    }

    {
        let mut app = handles.state.lock().await;
        if let Some(scan) = &mut app.scan_data {
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            for (name, ratio) in &results {
                if let Some(g) = scan.detailed_guilds.iter_mut().find(|g| &g.name == name) {
                    g.sim_win_ratio = Some(*ratio);
                    g.sim_mushrooms = Some(mushrooms);
                    g.sim_iterations = Some(iterations);
                    g.sim_evaluated_at = Some(now.clone());
                }
            }
            if let Err(e) = save_scan_data(scan) {
                log::error!("Failed to save scan data after simulation: {}", e);
            }
        }
    }

    let was_cancelled = handles.progress.lock().await.cancel_sim;
    {
        let mut p = handles.progress.lock().await;
        p.sim = SimProgress {
            running: false,
            phase: if was_cancelled { "aborted".into() } else { "done".into() },
            current: p.sim.current,
            total: p.sim.total,
            message: if was_cancelled {
                "Simulation abgebrochen.".into()
            } else {
                format!("Simulation abgeschlossen ({} Gilde(n)).", results.len())
            },
        };
        p.cancel_sim = false;
    }
}

/// POST /api/simulate/abort – Request cancellation of a running simulation
async fn abort_simulate(State(handles): State<AppHandles>) -> impl IntoResponse {
    let mut p = handles.progress.lock().await;
    if !p.sim.running {
        return ok_response("Keine laufende Simulation".to_string());
    }
    p.cancel_sim = true;
    ok_response("Abbruch angefordert".to_string())
}

/// GET /api/simulate/progress – Poll simulation progress
async fn get_sim_progress(State(handles): State<AppHandles>) -> impl IntoResponse {
    let p = handles.progress.lock().await;
    ok_response(p.sim.clone())
}

/// POST /api/refresh-own-fighters – Force-refresh the own-fighters cache,
/// ignoring TTL and roster-change detection.
async fn refresh_own_fighters_endpoint(State(handles): State<AppHandles>) -> impl IntoResponse {
    {
        let app = handles.state.lock().await;
        if app.selected.is_none() {
            return Err(err_response::<String>("Kein Charakter ausgewählt"));
        }
    }
    {
        let mut app = handles.state.lock().await;
        app.own_fighters_cache = None;
    }
    match get_own_fighters(&handles.state).await {
        Ok(fighters) => Ok(ok_response(format!(
            "{} eigene Kampfwerte aktualisiert",
            fighters.len()
        ))),
        Err(e) => Err(err_response::<String>(&e)),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Persistence
// ═══════════════════════════════════════════════════════════════════════════════

const DATA_DIR: &str = "/app/data";
const HISTORY_DIR: &str = "/app/data/history";

fn scan_file_path(server: &str, guild_name: &str) -> String {
    let server_clean = server
        .replace("https://", "")
        .replace("http://", "")
        .replace('/', "_")
        .replace(':', "_");
    let guild_clean = guild_name
        .replace(['/', '\\', ' '], "_");
    format!("{}/scan_{}_{}.json", DATA_DIR, server_clean, guild_clean)
}

fn history_file_path(server: &str, guild_name: &str, ts: &str) -> String {
    let server_clean = server
        .replace("https://", "")
        .replace("http://", "")
        .replace('/', "_")
        .replace(':', "_");
    let guild_clean = guild_name.replace(['/', '\\', ' '], "_");
    format!("{}/scan_{}_{}_{}.json", HISTORY_DIR, server_clean, guild_clean, ts)
}

fn save_scan_data(data: &ScanData) -> Result<(), String> {
    std::fs::create_dir_all(DATA_DIR).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(HISTORY_DIR).map_err(|e| e.to_string())?;

    let latest_path = scan_file_path(&data.server, &data.own_guild.name);
    let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;

    // latest (stable filename)
    std::fs::write(&latest_path, &json).map_err(|e| e.to_string())?;
    log::info!("Scan data saved to {}", latest_path);

    // history (timestamped)
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let hist_path = history_file_path(&data.server, &data.own_guild.name, &ts);
    std::fs::write(&hist_path, &json).map_err(|e| e.to_string())?;
    log::info!("Scan data archived to {}", hist_path);

    Ok(())
}

fn load_scan_data(server: &str, guild_name: &str) -> Result<ScanData, String> {
    let path = scan_file_path(server, guild_name);
    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&json).map_err(|e| e.to_string())
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Main
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .init();

    let state: SharedState = Arc::new(Mutex::new(AppState {
        sessions: Vec::new(),
        selected: None,
        own_guild: None,
        scan_data: None,
        scan_settings: ScanSettings::default(),
        own_fighters_cache: None,
        enemy_fighters_cache: HashMap::new(),
    }));
    let progress: SharedProgress = Arc::new(Mutex::new(ProgressState::default()));
    let handles = AppHandles { state, progress };

    let app = Router::new()
        .route("/api/login", post(login))
        .route("/api/characters", get(list_characters))
        .route("/api/select-character", post(select_character))
        .route("/api/scan", post(start_scan))
        .route("/api/scan/abort", post(abort_scan))
        .route("/api/progress", get(get_progress))
        .route("/api/results", post(get_results))
        .route("/api/guild-details", post(guild_details))
        .route("/api/simulate", post(start_simulate))
        .route("/api/simulate/abort", post(abort_simulate))
        .route("/api/simulate/progress", get(get_sim_progress))
        .route("/api/refresh-own-fighters", post(refresh_own_fighters_endpoint))
        .route("/api/status", get(status))
        .route("/api/logout", post(logout))
        .route("/api/export", get(export_data))
        .fallback_service(
            ServeDir::new("/app/static").append_index_html_on_directories(true),
        )
        .with_state(handles);

    let addr = "0.0.0.0:8080";
    log::info!("⚔️  SF Guild Scanner on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
