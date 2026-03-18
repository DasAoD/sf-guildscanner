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
}

// ── App State ────────────────────────────────────────────────────────────────

struct AppState {
    sessions: Vec<SimpleSession>,
    selected: Option<usize>,
    own_guild: Option<OwnGuild>,
    scan_data: Option<ScanData>,
    /// Progress tracking for long-running scans
    scan_progress: ScanProgress,
    /// Scan settings for the next run
    scan_settings: ScanSettings,
    /// Cancellation flag (set via /api/scan/abort)
    cancel_requested: bool,
}

#[derive(Serialize, Clone, Default)]
struct ScanProgress {
    running: bool,
    phase: String,        // "idle", "hof", "details", "done", "error"
    current: u32,
    total: u32,
    message: String,
}


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
}

impl Default for ScanSettings {
    fn default() -> Self {
        Self {
            down_limit: 300,
            honor_up_scan: true,
            max_extra_up_pages: 10,
            strict_mode: true,
            strict_topn: true,
        }
    }
}

type SharedState = Arc<Mutex<AppState>>;

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
}


#[derive(Deserialize)]
struct GuildDetailRequest {
    name: String,
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

#[derive(Serialize)]
struct FilteredResult {
    own_guild: OwnGuild,
    guilds: Vec<DetailedGuild>,
    /// Total attackable guilds before filtering
    total_attackable: usize,
    /// After filtering
    filtered_count: usize,
    scanned_at: String,
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
    State(state): State<SharedState>,
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
            let mut app = state.lock().await;
            app.sessions = sessions;
            app.selected = None;
            app.own_guild = None;
            app.scan_data = None;
            app.scan_progress = ScanProgress::default();

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

/// POST /api/select-character
async fn select_character(
    State(state): State<SharedState>,
    Json(req): Json<SelectCharRequest>,
) -> impl IntoResponse {
    let mut app = state.lock().await;

    if req.index >= app.sessions.len() {
        return Err(err_response::<OwnGuild>("Ungültiger Charakter-Index"));
    }

    app.selected = Some(req.index);

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
    State(state): State<SharedState>,
    Json(req): Json<ScanRequest>,
) -> impl IntoResponse {
    {
        let app = state.lock().await;
        if app.selected.is_none() || app.own_guild.is_none() {
            return Err(err_response::<String>("Kein Charakter/Gilde geladen"));
        }
        if app.scan_progress.running {
            return Err(err_response::<String>("Scan läuft bereits"));
        }
    }
    {
        let mut app = state.lock().await;
        app.cancel_requested = false;
        // Apply scan settings (with defaults)
        let mut s = app.scan_settings.clone();
        if let Some(v) = req.down_limit { s.down_limit = v; }
        if let Some(v) = req.honor_up_scan { s.honor_up_scan = v; }
        if let Some(v) = req.max_extra_up_pages { s.max_extra_up_pages = v; }
        if let Some(v) = req.strict_mode { s.strict_mode = v; }
        if let Some(v) = req.strict_topn { s.strict_topn = v; }
        // clamp to sane values
        s.down_limit = s.down_limit.clamp(0, 50_000);
        s.max_extra_up_pages = s.max_extra_up_pages.clamp(0, 50);
        app.scan_settings = s;
    }

    // Start scan in background task
    let state_clone = state.clone();
    tokio::spawn(async move {
        if let Err(e) = run_scan(state_clone.clone()).await {
            log::error!("Scan error: {}", e);
            let mut app = state_clone.lock().await;
            app.scan_progress = ScanProgress {
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
async fn abort_scan(State(state): State<SharedState>) -> impl IntoResponse {
    let mut app = state.lock().await;

    if !app.scan_progress.running {
        return ok_response("Kein laufender Scan".to_string());
    }

    app.cancel_requested = true;
    ok_response("Abbruch angefordert".to_string())
}

/// The actual scan logic, running as a background task
async fn run_scan(state: SharedState) -> Result<(), String> {
    const PAGE_SIZE: u32 = 51;

    // Snapshot required state + reset cancel flag
    let (own_guild, server, settings) = {
        let mut app = state.lock().await;
        app.cancel_requested = false;
        app.scan_progress = ScanProgress {
            running: true,
            phase: "hof".into(),
            current: 0,
            total: 0,
            message: "Starte HoF-Scan...".into(),
        };
        let og = app.own_guild.clone().unwrap();
        let idx = app.selected.unwrap();
        let server = app.sessions[idx].server_url().to_string();
        let settings = app.scan_settings.clone();
        (og, server, settings)
    };

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
    async fn cancelled(state: &SharedState) -> bool {
        state.lock().await.cancel_requested
    }

    let mut all_hof_guilds: Vec<HofGuild> = Vec::new();

    // ── 1a) Scan mandatory rank-window pages (includes up to 20 above + down_limit below)
    let total_pages = page_high.saturating_sub(page_low) + 1;
    {
        let mut app = state.lock().await;
        app.scan_progress.total = total_pages;
        app.scan_progress.message = format!(
            "Scanne Rangbereich #{}..#{} ({} Seiten)...",
            rank_up_start,
            rank_down_end,
            total_pages
        );
    }

    for (i, page) in (page_low..=page_high).enumerate() {
        if cancelled(&state).await {
            log::info!("HoF scan cancelled during window pages");
            break;
        }

        {
            let mut app = state.lock().await;
            app.scan_progress.current = (i as u32) + 1;
            app.scan_progress.message = format!(
                "HoF Seite {} wird gescannt... ({} Gilden gesammelt)",
                page + 1,
                all_hof_guilds.len()
            );
        }

        let guilds_on_page = {
            let mut app = state.lock().await;
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

    // ── 1b) Optional honor-up-scan: scan extra pages ABOVE the 20-rank window until honor-rule yields nothing
    if settings.honor_up_scan && page_low > 0 && !cancelled(&state).await {
        let mut extra_scanned = 0u32;
        let mut page = page_low - 1;

        loop {
            if cancelled(&state).await {
                log::info!("HoF scan cancelled during honor-up pages");
                break;
            }
            if extra_scanned >= settings.max_extra_up_pages {
                break;
            }

            {
                let mut app = state.lock().await;
                app.scan_progress.message = format!(
                    "HoF (Ehre-Regel): Seite {} wird geprüft...",
                    page + 1
                );
            }

            let guilds_on_page = {
            let mut app = state.lock().await;
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

            let mut found_any = false;
            for hg in &guilds_on_page {
                if hg.name == own_guild.name {
                    continue;
                }

                // Only above us
                if hg.rank >= own_rank {
                    continue;
                }

                // Honor-rule: guild honor is at most 3000 above ours (or lower)
                if hg.honor > own_honor.saturating_add(3000) {
                    continue;
                }

                found_any = true;

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

            // Stop as soon as a page yields no honor-rule candidates
            if !found_any {
                break;
            }

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
        let mut app = state.lock().await;
        app.scan_progress.phase = "details".into();
        app.scan_progress.current = 0;
        app.scan_progress.total = attackable_count as u32;
        app.scan_progress.message = format!(
            "Phase 2: Lade Details für {} angreifbare Gilden...",
            attackable_count
        );
    }

    let mut detailed_guilds: Vec<DetailedGuild> = Vec::new();

    for (i, (name, rank, honor, is_attacked)) in attackable_names.iter().enumerate() {
        if cancelled(&state).await {
            log::info!("Details loading cancelled");
            break;
        }

        {
            let mut app = state.lock().await;
            app.scan_progress.current = i as u32 + 1;
            app.scan_progress.message = format!(
                "Lade Gilde {}/{}: {}",
                i + 1,
                attackable_count,
                name
            );
        }

        let detail = {
            let mut app = state.lock().await;
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
                            strict_evaluated,
                            strict_beatable,
                            strict_own_active_members,
                            strict_topn,
                            strict_topn_n,
                            strict_fail_index,
                            strict_fail_enemy_level,
                            strict_fail_own_level,
                            strict_fail_reason,
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

    let was_cancelled = cancelled(&state).await;

    {
        let mut app = state.lock().await;
        app.scan_data = Some(scan_data);
        app.scan_progress = ScanProgress {
            running: false,
            phase: if was_cancelled { "aborted".into() } else { "done".into() },
            current: app.scan_progress.current,
            total: app.scan_progress.total,
            message: if was_cancelled {
                "Scan abgebrochen – Teilergebnis gespeichert.".into()
            } else {
                "Scan abgeschlossen.".into()
            },
        };
        app.cancel_requested = false;
    }

    Ok(())
}

/// GET /api/progress – Poll scan progress
async fn get_progress(State(state): State<SharedState>) -> impl IntoResponse {
    let app = state.lock().await;
    ok_response(app.scan_progress.clone())
}

/// POST /api/results – Get filtered results
async fn get_results(
    State(state): State<SharedState>,
    Json(filter): Json<FilterRequest>,
) -> impl IntoResponse {
    let app = state.lock().await;

    let scan = match &app.scan_data {
        Some(s) => s,
        None => return Err(err_response::<FilteredResult>("Keine Scan-Daten vorhanden")),
    };

    let total_attackable = scan.detailed_guilds.len();
    let strict_available = scan.detailed_guilds.iter().any(|g| g.strict_evaluated);
    // "winnable_only" default: if strict data exists, default to strict-only.
    let strict_only = filter.strict_only.unwrap_or(true);

    let filtered: Vec<DetailedGuild> = scan
        .detailed_guilds
        .iter()
        .filter(|g| {
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

    Ok(ok_response(FilteredResult {
        own_guild: scan.own_guild.clone(),
        guilds: filtered,
        total_attackable,
        filtered_count,
        scanned_at: scan.scanned_at.clone(),
    }))
}

/// POST /api/guild-details – View details of a specific guild (live request)
async fn guild_details(
    State(state): State<SharedState>,
    Json(req): Json<GuildDetailRequest>,
) -> impl IntoResponse {
    // First check if we already have it in scan data
    {
        let app = state.lock().await;
        if let Some(scan) = &app.scan_data {
            if let Some(guild) = scan.detailed_guilds.iter().find(|g| g.name == req.name) {
                return Ok(ok_response(guild.clone()));
            }
        }
    }

    // Otherwise fetch live
    let mut app = state.lock().await;
    let idx = match app.selected {
        Some(i) => i,
        None => return Err(err_response::<DetailedGuild>("Kein Charakter ausgewählt")),
    };

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
                    strict_evaluated: false,
                    strict_beatable: false,
                    strict_own_active_members: 0,
                    strict_topn: false,
                    strict_topn_n: 0,
                    strict_fail_index: None,
                    strict_fail_enemy_level: None,
                    strict_fail_own_level: None,
                    strict_fail_reason: None,
                }))
            } else {
                Err(err_response::<DetailedGuild>("Gilde nicht gefunden"))
            }
        }
        Err(e) => Err(err_response::<DetailedGuild>(&format!("Fehler: {:?}", e))),
    }
}

/// GET /api/status
async fn status(State(state): State<SharedState>) -> impl IntoResponse {
    let app = state.lock().await;

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
async fn logout(State(state): State<SharedState>) -> impl IntoResponse {
    let mut app = state.lock().await;
    app.sessions.clear();
    app.selected = None;
    app.own_guild = None;
    app.scan_data = None;
    app.scan_progress = ScanProgress::default();
    ok_response("Ausgeloggt")
}

/// GET /api/export – Export scan data as JSON download
async fn export_data(State(state): State<SharedState>) -> impl IntoResponse {
    let app = state.lock().await;
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
        scan_progress: ScanProgress::default(),
        scan_settings: ScanSettings::default(),
        cancel_requested: false,
    }));

    let app = Router::new()
        .route("/api/login", post(login))
        .route("/api/select-character", post(select_character))
        .route("/api/scan", post(start_scan))
        .route("/api/scan/abort", post(abort_scan))
        .route("/api/progress", get(get_progress))
        .route("/api/results", post(get_results))
        .route("/api/guild-details", post(guild_details))
        .route("/api/status", get(status))
        .route("/api/logout", post(logout))
        .route("/api/export", get(export_data))
        .fallback_service(
            ServeDir::new("/app/static").append_index_html_on_directories(true),
        )
        .with_state(state);

    let addr = "0.0.0.0:8080";
    log::info!("⚔️  SF Guild Scanner on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
