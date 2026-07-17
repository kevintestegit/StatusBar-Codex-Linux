use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::{CString, c_char, c_int, c_void};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

mod config;
mod mascot;
mod paths;
mod pricing;
mod status;

use config::*;
use paths::*;
use pricing::*;
use status::{CodexStatus, StatusSource, rate_limits_still_valid, resolve_codex_status};

const PRIMARY_WINDOW_SECONDS: i64 = 5 * 60 * 60;
const PACE_ALERT_AHEAD_PERCENT: f64 = 10.0;
const PARTY_OVERLAY_COOLDOWN_SECONDS: i64 = 10 * 60;

fn debug_log(msg: &str) {
    if env::var_os("CODEX_USAGE_TRAY_DEBUG").is_none() {
        return;
    }
    let _ = writeln!(io::stderr(), "[StatusBar-Codex-Linux] {msg}");
}

#[derive(Clone, Default)]
struct Usage {
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
    reasoning_output_tokens: i64,
    total_tokens: i64,
}

impl Usage {
    fn add_json(&mut self, value: &Value) {
        self.input_tokens += value_i64(value, "input_tokens");
        self.cached_input_tokens += value_i64(value, "cached_input_tokens");
        self.output_tokens += value_i64(value, "output_tokens");
        self.reasoning_output_tokens += value_i64(value, "reasoning_output_tokens");
        self.total_tokens += value_i64(value, "total_tokens");
    }

    fn merge(&mut self, other: &Usage) {
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
        self.total_tokens += other.total_tokens;
    }
}

#[derive(Clone, Default)]
pub struct WindowLimit {
    pub used_percent: f64,
    pub resets_at: Option<i64>,
    pub window_duration_mins: Option<i64>,
}

#[derive(Clone, Default)]
pub struct RateLimits {
    pub plan_type: String,
    pub primary: WindowLimit,
    pub secondary: WindowLimit,
}

#[derive(Clone, Default)]
struct Stats {
    total: Usage,
    today: Usage,
    month: Usage,
    by_model: HashMap<String, Usage>,
    today_by_model: HashMap<String, Usage>,
    month_by_model: HashMap<String, Usage>,
    rate_limits: Option<RateLimits>,
    latest_rate_ts: Option<DateTime<Local>>,
    files_seen: usize,
    events_seen: usize,
    status_source: StatusSource,
    auth_status: String,
    status_fetched_at: Option<i64>,
    status_error: Option<String>,
}

#[derive(Clone, Default)]
struct FileStats {
    total: Usage,
    by_model: HashMap<String, Usage>,
    by_day: HashMap<NaiveDate, Usage>,
    by_day_model: HashMap<NaiveDate, HashMap<String, Usage>>,
    by_month: HashMap<(i32, u32), Usage>,
    by_month_model: HashMap<(i32, u32), HashMap<String, Usage>>,
    rate_limits: Option<RateLimits>,
    latest_rate_ts: Option<DateTime<Local>>,
    events_seen: usize,
}

struct CachedFile {
    len: u64,
    modified_ns: u128,
    stats: FileStats,
}

#[derive(Default)]
struct StatsCache {
    files: HashMap<PathBuf, CachedFile>,
    last_stats: Option<Stats>,
    last_day: Option<NaiveDate>,
    last_month: Option<(i32, u32)>,
}

#[repr(C)]
struct GtkWidget(c_void);
#[repr(C)]
struct GtkMenu(c_void);
#[repr(C)]
struct AppIndicator(c_void);
#[repr(C)]
struct GtkCssProvider(c_void);
#[repr(C)]
struct GdkScreen(c_void);
#[repr(C)]
struct GdkVisual(c_void);
#[repr(C)]
struct Cairo(c_void);
#[repr(C)]
struct CairoRegion(c_void);

#[link(name = "gtk-3")]
unsafe extern "C" {
    fn gtk_init(argc: *mut c_int, argv: *mut *mut *mut c_char);
    fn gtk_main();
    fn gtk_main_quit();
    fn gtk_menu_new() -> *mut GtkWidget;
    fn gtk_menu_item_new() -> *mut GtkWidget;
    fn gtk_menu_item_new_with_label(label: *const c_char) -> *mut GtkWidget;
    fn gtk_menu_item_set_submenu(menu_item: *mut GtkWidget, submenu: *mut GtkWidget);
    fn gtk_separator_menu_item_new() -> *mut GtkWidget;
    fn gtk_menu_shell_append(menu_shell: *mut GtkWidget, child: *mut GtkWidget);
    fn gtk_widget_show_all(widget: *mut GtkWidget);
    fn gtk_widget_set_sensitive(widget: *mut GtkWidget, sensitive: c_int);
    fn gtk_widget_destroy(widget: *mut GtkWidget);
    fn gtk_widget_queue_draw(widget: *mut GtkWidget);
    fn gtk_widget_set_size_request(widget: *mut GtkWidget, width: c_int, height: c_int);
    fn gtk_widget_set_app_paintable(widget: *mut GtkWidget, app_paintable: c_int);
    fn gtk_widget_set_opacity(widget: *mut GtkWidget, opacity: f64);
    fn gtk_widget_set_halign(widget: *mut GtkWidget, align: c_int);
    fn gtk_widget_set_valign(widget: *mut GtkWidget, align: c_int);
    fn gtk_widget_get_screen(widget: *mut GtkWidget) -> *mut GdkScreen;
    fn gtk_widget_set_visual(widget: *mut GtkWidget, visual: *mut GdkVisual);
    fn gtk_widget_input_shape_combine_region(widget: *mut GtkWidget, region: *mut CairoRegion);
    fn gtk_widget_get_allocated_width(widget: *mut GtkWidget) -> c_int;
    fn gtk_widget_get_allocated_height(widget: *mut GtkWidget) -> c_int;
    fn gtk_label_new(str: *const c_char) -> *mut GtkWidget;
    fn gtk_label_set_markup(label: *mut GtkWidget, str: *const c_char);
    fn gtk_label_set_xalign(label: *mut GtkWidget, xalign: f32);
    fn gtk_container_add(container: *mut GtkWidget, widget: *mut GtkWidget);
    fn gtk_overlay_new() -> *mut GtkWidget;
    fn gtk_overlay_add_overlay(overlay: *mut GtkWidget, widget: *mut GtkWidget);
    fn gtk_drawing_area_new() -> *mut GtkWidget;
    fn gtk_window_new(window_type: c_int) -> *mut GtkWidget;
    fn gtk_window_set_title(window: *mut GtkWidget, title: *const c_char);
    fn gtk_window_set_default_size(window: *mut GtkWidget, width: c_int, height: c_int);
    fn gtk_window_set_decorated(window: *mut GtkWidget, setting: c_int);
    fn gtk_window_set_keep_above(window: *mut GtkWidget, setting: c_int);
    fn gtk_css_provider_new() -> *mut GtkCssProvider;
    fn gtk_css_provider_load_from_data(
        css_provider: *mut GtkCssProvider,
        data: *const c_char,
        length: isize,
        error: *mut *mut c_void,
    ) -> c_int;
    fn gtk_style_context_add_provider_for_screen(
        screen: *mut GdkScreen,
        provider: *mut GtkCssProvider,
        priority: u32,
    );
}

#[link(name = "gdk-3")]
unsafe extern "C" {
    fn gdk_screen_get_rgba_visual(screen: *mut GdkScreen) -> *mut GdkVisual;
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_signal_connect_data(
        instance: *mut c_void,
        detailed_signal: *const c_char,
        c_handler: *mut c_void,
        data: *mut c_void,
        destroy_data: *mut c_void,
        connect_flags: c_int,
    ) -> u64;
}

#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_timeout_add_seconds(
        interval: u32,
        function: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
        data: *mut c_void,
    ) -> u32;
    fn g_timeout_add(
        interval: u32,
        function: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
        data: *mut c_void,
    ) -> u32;
}

#[link(name = "ayatana-appindicator3")]
unsafe extern "C" {
    fn app_indicator_new(
        id: *const c_char,
        icon_name: *const c_char,
        category: c_int,
    ) -> *mut AppIndicator;
    fn app_indicator_set_status(self_: *mut AppIndicator, status: c_int);
    fn app_indicator_set_menu(self_: *mut AppIndicator, menu: *mut GtkMenu);
    fn app_indicator_set_label(
        self_: *mut AppIndicator,
        label: *const c_char,
        guide: *const c_char,
    );
    fn app_indicator_set_icon_full(
        self_: *mut AppIndicator,
        icon_name: *const c_char,
        icon_desc: *const c_char,
    );
    fn app_indicator_set_title(self_: *mut AppIndicator, title: *const c_char);
    fn app_indicator_set_icon_theme_path(self_: *mut AppIndicator, path: *const c_char);
}

#[link(name = "gtk-layer-shell")]
unsafe extern "C" {
    fn gtk_layer_init_for_window(window: *mut GtkWidget);
    fn gtk_layer_set_namespace(window: *mut GtkWidget, name_space: *const c_char);
    fn gtk_layer_set_layer(window: *mut GtkWidget, layer: c_int);
    fn gtk_layer_set_anchor(window: *mut GtkWidget, edge: c_int, anchor_to_edge: c_int);
    fn gtk_layer_set_margin(window: *mut GtkWidget, edge: c_int, margin_size: c_int);
    fn gtk_layer_set_exclusive_zone(window: *mut GtkWidget, exclusive_zone: c_int);
    fn gtk_layer_set_keyboard_mode(window: *mut GtkWidget, mode: c_int);
}

#[link(name = "cairo")]
unsafe extern "C" {
    fn cairo_region_create() -> *mut CairoRegion;
    fn cairo_region_destroy(region: *mut CairoRegion);
    fn cairo_save(cr: *mut Cairo);
    fn cairo_restore(cr: *mut Cairo);
    fn cairo_set_source_rgba(cr: *mut Cairo, r: f64, g: f64, b: f64, a: f64);
    fn cairo_set_operator(cr: *mut Cairo, op: c_int);
    fn cairo_paint(cr: *mut Cairo);
    fn cairo_rectangle(cr: *mut Cairo, x: f64, y: f64, width: f64, height: f64);
    fn cairo_arc(cr: *mut Cairo, xc: f64, yc: f64, radius: f64, angle1: f64, angle2: f64);
    fn cairo_fill(cr: *mut Cairo);
    fn cairo_translate(cr: *mut Cairo, tx: f64, ty: f64);
    fn cairo_rotate(cr: *mut Cairo, angle: f64);
}

struct AppState {
    indicator: *mut AppIndicator,
    primary_header_label: *mut GtkWidget,
    limit_label: *mut GtkWidget,
    weekly_header_label: *mut GtkWidget,
    weekly_label: *mut GtkWidget,
    pace_label: *mut GtkWidget,
    plan_label: *mut GtkWidget,
    auth_label: *mut GtkWidget,
    updated_label: *mut GtkWidget,
    source_label: *mut GtkWidget,
    party_mode_label: *mut GtkWidget,
    mascot_label: *mut GtkWidget,
    refresh_interval_label: *mut GtkWidget,
    last_render: Option<RenderSnapshot>,
    last_refresh_at: Option<i64>,
    seen_primary_window: bool,
    last_primary_resets_at: Option<i64>,
    last_primary_reset_notified_at: Option<i64>,
    pace_alert_active: bool,
    pace_alert_window: Option<i64>,
    seen_secondary_window: bool,
    last_secondary_resets_at: Option<i64>,
}

#[derive(Clone, PartialEq)]
struct RenderSnapshot {
    primary_header_markup: String,
    limit_markup: String,
    weekly_header_markup: String,
    weekly_markup: String,
    pace_markup: String,
    plan_markup: String,
    auth_markup: String,
    updated_markup: String,
    source_markup: String,
    party_mode_markup: String,
    mascot_markup: String,
    refresh_interval_markup: String,
    tray_label: String,
    svg_label: String,
    title: String,
}

unsafe impl Send for AppState {}

static STATE: OnceLock<Mutex<AppState>> = OnceLock::new();
static STATS_CACHE: OnceLock<Mutex<StatsCache>> = OnceLock::new();
static LAST_PARTY_OVERLAY_AT: OnceLock<Mutex<Option<i64>>> = OnceLock::new();

fn value_i64(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn cost_for_usage(model: &str, usage: &Usage) -> Option<f64> {
    let p = price(model)?;
    let cached = usage.cached_input_tokens.max(0) as f64;
    let uncached = (usage.input_tokens - usage.cached_input_tokens).max(0) as f64;
    Some(
        (uncached * p.input + cached * p.cached_input + usage.output_tokens as f64 * p.output)
            / 1_000_000.0,
    )
}

fn sum_cost(models: &HashMap<String, Usage>) -> Option<f64> {
    let mut total = 0.0;
    let mut any = false;
    for (model, usage) in models {
        if let Some(cost) = cost_for_usage(model, usage) {
            total += cost;
            any = true;
        }
    }
    any.then_some(total)
}

fn unpriced_tokens(stats: &Stats) -> i64 {
    stats
        .by_model
        .iter()
        .filter(|(model, _)| price(model).is_none())
        .map(|(_, usage)| usage.total_tokens)
        .sum()
}

fn collect_stats() -> Stats {
    let cache = STATS_CACHE.get_or_init(|| Mutex::new(StatsCache::default()));
    let mut cache = cache.lock().unwrap();
    let mut stats = collect_stats_cached(&mut cache);
    let jsonl_limits = stats.rate_limits.clone().filter(rate_limits_still_valid);
    if stats.rate_limits.is_some() && jsonl_limits.is_none() {
        debug_log("local JSONL rate limits expired/stale; not treating as current status");
    }
    let status = resolve_codex_status(jsonl_limits);
    apply_status(&mut stats, status);
    stats
}

fn apply_status(stats: &mut Stats, status: CodexStatus) {
    stats.rate_limits = status.rate_limits;
    stats.status_source = status.source;
    stats.auth_status = status.auth_status;
    stats.status_fetched_at = status.fetched_at;
    stats.status_error = status.error;
}

fn collect_stats_cached(cache: &mut StatsCache) -> Stats {
    let now = Local::now();
    let today = now.date_naive();
    let month = (now.year(), now.month());
    let mut changed = cache.last_day != Some(today) || cache.last_month != Some(month);
    let mut seen = HashSet::new();
    let codex_home = codex_home();
    for root in [
        codex_home.join("sessions"),
        codex_home.join("archived_sessions"),
    ] {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|s| s.to_str()) != Some("jsonl")
            {
                continue;
            }
            let path = entry.path().to_path_buf();
            seen.insert(path.clone());
            let Some((len, modified_ns)) = file_key(&path) else {
                continue;
            };
            let stale = cache
                .files
                .get(&path)
                .is_none_or(|cached| cached.len != len || cached.modified_ns != modified_ns);
            if stale {
                changed = true;
                cache.files.insert(
                    path,
                    CachedFile {
                        len,
                        modified_ns,
                        stats: parse_file_stats(entry.path()),
                    },
                );
            }
        }
    }
    let before_retain = cache.files.len();
    cache.files.retain(|path, _| seen.contains(path));
    if cache.files.len() != before_retain {
        changed = true;
    }
    if !changed && let Some(stats) = cache.last_stats.clone() {
        return stats;
    }
    let stats = aggregate_cached_stats(cache, today, month);
    cache.last_day = Some(today);
    cache.last_month = Some(month);
    cache.last_stats = Some(stats.clone());
    stats
}

fn file_key(path: &Path) -> Option<(u64, u128)> {
    let metadata = fs::metadata(path).ok()?;
    let modified_ns = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((metadata.len(), modified_ns))
}

fn aggregate_cached_stats(cache: &StatsCache, today: NaiveDate, month: (i32, u32)) -> Stats {
    let mut stats = Stats::default();
    for cached in cache.files.values() {
        let file = &cached.stats;
        stats.files_seen += 1;
        stats.events_seen += file.events_seen;
        stats.total.merge(&file.total);
        merge_usage_map(&mut stats.by_model, &file.by_model);
        if let Some(day) = file.by_day.get(&today) {
            stats.today.merge(day);
        }
        if let Some(day_models) = file.by_day_model.get(&today) {
            merge_usage_map(&mut stats.today_by_model, day_models);
        }
        if let Some(month_usage) = file.by_month.get(&month) {
            stats.month.merge(month_usage);
        }
        if let Some(month_models) = file.by_month_model.get(&month) {
            merge_usage_map(&mut stats.month_by_model, month_models);
        }
        if let (Some(ts), Some(rate_limits)) = (file.latest_rate_ts, file.rate_limits.clone())
            && stats.latest_rate_ts.is_none_or(|latest| ts > latest)
        {
            stats.latest_rate_ts = Some(ts);
            stats.rate_limits = Some(rate_limits);
        }
    }
    stats
}

fn merge_usage_map(target: &mut HashMap<String, Usage>, source: &HashMap<String, Usage>) {
    for (model, usage) in source {
        target.entry(model.clone()).or_default().merge(usage);
    }
}

fn parse_file_stats(path: &Path) -> FileStats {
    let mut stats = FileStats::default();
    let Ok(content) = fs::read_to_string(path) else {
        return stats;
    };
    let mut model = "unknown".to_string();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let typ = obj.get("type").and_then(Value::as_str).unwrap_or("");
        let payload = obj.get("payload").unwrap_or(&Value::Null);
        if typ == "session_meta" || typ == "turn_context" {
            if let Some(new_model) = payload.get("model").and_then(Value::as_str) {
                model = normalize_model(new_model);
            }
            continue;
        }
        if typ != "event_msg" || payload.get("type").and_then(Value::as_str) != Some("token_count")
        {
            continue;
        }
        let ts = obj
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
            .unwrap_or_else(Local::now);

        // Rate-limit snapshot is independent of token totals. Prefer latest.
        if let Some(rate_limits) = parse_rate_limits(payload)
            && stats.latest_rate_ts.is_none_or(|latest| ts > latest)
        {
            stats.latest_rate_ts = Some(ts);
            stats.rate_limits = Some(rate_limits);
        }

        // Historical token aggregation (not used for tray rate-limit status).
        let Some(usage_json) = payload.pointer("/info/last_token_usage") else {
            continue;
        };
        let mut usage = Usage::default();
        usage.add_json(usage_json);
        stats.events_seen += 1;
        stats.total.merge(&usage);
        stats
            .by_model
            .entry(model.clone())
            .or_default()
            .merge(&usage);
        let day = ts.date_naive();
        stats.by_day.entry(day).or_default().merge(&usage);
        stats
            .by_day_model
            .entry(day)
            .or_default()
            .entry(model.clone())
            .or_default()
            .merge(&usage);
        let month = (ts.year(), ts.month());
        stats.by_month.entry(month).or_default().merge(&usage);
        stats
            .by_month_model
            .entry(month)
            .or_default()
            .entry(model.clone())
            .or_default()
            .merge(&usage);
    }
    stats
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Local))
}

fn parse_rate_limits(payload: &Value) -> Option<RateLimits> {
    let raw = payload.get("rate_limits")?;
    Some(RateLimits {
        plan_type: raw
            .get("plan_type")
            .and_then(Value::as_str)
            .unwrap_or("n/a")
            .to_string(),
        primary: parse_window(raw.get("primary")),
        secondary: parse_window(raw.get("secondary")),
    })
}

fn parse_window(value: Option<&Value>) -> WindowLimit {
    let Some(value) = value else {
        return WindowLimit::default();
    };
    WindowLimit {
        used_percent: value
            .get("used_percent")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        resets_at: value.get("resets_at").and_then(Value::as_i64),
        window_duration_mins: value
            .get("windowDurationMins")
            .and_then(Value::as_i64)
            .or_else(|| value.get("window_duration_mins").and_then(Value::as_i64)),
    }
}

fn window_present(w: &WindowLimit) -> bool {
    w.resets_at.is_some()
}

// ponytail: label by the real window duration the API reports, not a hardcoded "5h"/"W".
fn window_short(mins: Option<i64>) -> &'static str {
    match mins {
        Some(300) => "5h",
        Some(10080) => "W",
        Some(m) if m >= 1440 => "W",
        _ => "lim",
    }
}

fn window_full(mins: Option<i64>) -> &'static str {
    match mins {
        Some(300) => "5h",
        Some(10080) => "Weekly",
        Some(m) if m >= 1440 => "Weekly",
        _ => "Limit",
    }
}

fn dollars(value: Option<f64>) -> String {
    match value {
        None => "n/a".into(),
        Some(v) if v >= 1000.0 => format!("${}", comma_int(v.round() as i64)),
        Some(v) if v >= 100.0 => format!("${}", comma_decimal(v, 1)),
        Some(v) => format!("${v:.2}"),
    }
}

fn full_tokens(value: i64) -> String {
    comma_int(value)
}

fn compact_tokens(value: i64) -> String {
    let abs = value.abs() as f64;
    if abs >= 1_000_000_000.0 {
        format!("{:.2}B", value as f64 / 1_000_000_000.0)
    } else if abs >= 1_000_000.0 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else {
        comma_int(value)
    }
}

fn comma_int(value: i64) -> String {
    let negative = value < 0;
    let digits = value.abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    let mut result: String = out.chars().rev().collect();
    if negative {
        result.insert(0, '-');
    }
    result
}

fn comma_decimal(value: f64, decimals: usize) -> String {
    let rounded = format!("{value:.decimals$}");
    let Some((whole, frac)) = rounded.split_once('.') else {
        return comma_int(value.round() as i64);
    };
    let whole = whole.parse::<i64>().unwrap_or(0);
    format!("{}.{}", comma_int(whole), frac)
}

fn display_plan(plan: &str) -> &str {
    if plan == "prolite" {
        "$100 Pro (Pro Lite)"
    } else {
        plan
    }
}

fn reset_text(resets_at: Option<i64>) -> String {
    let Some(reset) = resets_at else {
        return "n/a".into();
    };
    let Some(reset_dt) = Utc.timestamp_opt(reset, 0).single() else {
        return "n/a".into();
    };
    let seconds = reset_dt.signed_duration_since(Utc::now()).num_seconds();
    if seconds <= 0 {
        "now".into()
    } else {
        let days = seconds / 86_400;
        let hours = (seconds % 86_400) / 3_600;
        let minutes = (seconds % 3_600) / 60;
        if days > 0 {
            format!("{days}d {hours}h")
        } else if hours > 0 {
            format!("{hours}h {minutes}m")
        } else {
            format!("{minutes}m")
        }
    }
}

fn reset_clock_text(resets_at: Option<i64>) -> String {
    let Some(reset) = resets_at else {
        return "n/a".into();
    };
    let Some(reset_dt) = Local.timestamp_opt(reset, 0).single() else {
        return "n/a".into();
    };
    reset_dt.format("%H:%M").to_string()
}

#[derive(Clone, Copy)]
struct Pace {
    expected_percent: f64,
    ahead_percent: f64,
}

fn primary_pace(limit: &WindowLimit) -> Option<Pace> {
    let reset = limit.resets_at?;
    // ponytail: use the real window length the API reports, not a hardcoded 5h.
    let window_secs = limit.window_duration_mins.unwrap_or(300).max(1) as i64 * 60;
    let now = Utc::now().timestamp();
    let seconds_left = (reset - now).clamp(0, window_secs);
    let elapsed = window_secs - seconds_left;
    let expected_percent = elapsed as f64 * 100.0 / window_secs as f64;
    Some(Pace {
        expected_percent,
        ahead_percent: limit.used_percent - expected_percent,
    })
}

fn pace_text(limit: &WindowLimit) -> String {
    let Some(pace) = primary_pace(limit) else {
        return "n/a".into();
    };
    let diff = pace.ahead_percent.abs();
    if pace.ahead_percent >= PACE_ALERT_AHEAD_PERCENT {
        format!("fast by {:.1}%", diff)
    } else if pace.ahead_percent > 1.0 {
        format!("ahead by {:.1}%", diff)
    } else if pace.ahead_percent < -1.0 {
        format!("slow by {:.1}%", diff)
    } else {
        "on pace".into()
    }
}

fn pace_delta_markup(limit: &WindowLimit) -> String {
    let Some(pace) = primary_pace(limit) else {
        return "<b>Pace:</b>  n/a".into();
    };
    let diff = pace.ahead_percent.abs();
    if diff < 0.5 {
        "<b>Pace:</b>  on pace".into()
    } else if pace.ahead_percent > 0.0 {
        format!("<b>Pace:</b>  {:.1}% ↑ faster than expected", diff)
    } else {
        format!("<b>Pace:</b>  {:.1}% ↓ slower than expected", diff)
    }
}

fn usage_bar(percent: f64) -> String {
    let filled = ((percent.clamp(0.0, 100.0) / 10.0).round() as usize).clamp(0, 10);
    let empty = 10 - filled;
    format!("{}{}", "▰".repeat(filled), "▱".repeat(empty))
}

// ponytail: never invent 0% after resets_at — that hid stale data as "fresh zero".
fn display_rate_limits(rate: &RateLimits) -> Option<RateLimits> {
    if rate_limits_still_valid(rate) {
        Some(rate.clone())
    } else {
        None
    }
}

fn current_month_range_text() -> String {
    let now = Local::now();
    let year = now.year();
    let month = now.month();
    let Some(start) = chrono::NaiveDate::from_ymd_opt(year, month, 1) else {
        return "Current month".into();
    };
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let Some(next_start) = chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1) else {
        return "Current month".into();
    };
    let end = next_start - chrono::Duration::days(1);
    format!("{} - {}", start.format("%b %-d"), end.format("%b %-d, %Y"))
}

fn make_details_text(stats: &Stats) -> String {
    let total_cost = sum_cost(&stats.by_model);
    let today_cost = sum_cost(&stats.today_by_model);
    let month_cost = sum_cost(&stats.month_by_model);
    let rate = stats.rate_limits.clone().and_then(|r| display_rate_limits(&r));
    let config = load_config();
    let mut top: Vec<_> = stats.by_model.iter().collect();
    top.sort_by_key(|(_, usage)| -usage.total_tokens);
    let model_lines = top
        .into_iter()
        .take(8)
        .map(|(model, usage)| {
            format!(
                "{model}: {} tokens | input {} | cached {} | output {} | reasoning {} | cost {}",
                full_tokens(usage.total_tokens),
                full_tokens(usage.input_tokens),
                full_tokens(usage.cached_input_tokens),
                full_tokens(usage.output_tokens),
                full_tokens(usage.reasoning_output_tokens),
                dollars(cost_for_usage(model, usage))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let rate_block = match &rate {
        Some(rate) => format!(
            "Rate limits (Codex/ChatGPT account)\n{}: {:.0}% used | reset in {} at {}\n{}: {:.0}% used | reset in {} at {}\nPace: {} | expected {:.1}%",
            window_short(rate.primary.window_duration_mins),
            rate.primary.used_percent,
            reset_text(rate.primary.resets_at),
            reset_clock_text(rate.primary.resets_at),
            window_short(rate.secondary.window_duration_mins),
            rate.secondary.used_percent,
            reset_text(rate.secondary.resets_at),
            reset_clock_text(rate.secondary.resets_at),
            pace_text(&rate.primary),
            primary_pace(&rate.primary)
                .map(|pace| pace.expected_percent)
                .unwrap_or(0.0),
        ),
        None => "Rate limits\nCodex status unavailable (no recent account snapshot)".into(),
    };
    let plan = rate
        .as_ref()
        .map(|r| display_plan(&r.plan_type).to_string())
        .unwrap_or_else(|| "n/a".into());
    format!(
        "Codex Status\n\nPlan: {}\nAuth: {}\nStatus source: {}\nLast update: {}\nParty mode: {}\nRefresh interval: {}\n\n{}\n\nLocal Codex session tokens (not OpenCode)\nToday: {} tokens | {}\nThis month: {} tokens | {}\nAll-time: {} tokens | {}\n\nToken breakdown\nAll-time input: {}\nAll-time cached input: {}\nAll-time output: {}\nAll-time reasoning: {}\nSkipped with no public API price: {}\n\nModels\n{}\n\nJSONL events: {} from {} files\n",
        plan,
        stats.auth_status,
        stats.status_source.label(),
        fetched_at_text(stats.status_fetched_at),
        if config.party_mode { "on" } else { "off" },
        duration_label(config.refresh_seconds),
        rate_block,
        full_tokens(stats.today.total_tokens),
        dollars(today_cost),
        full_tokens(stats.month.total_tokens),
        dollars(month_cost),
        full_tokens(stats.total.total_tokens),
        dollars(total_cost),
        full_tokens(stats.total.input_tokens),
        full_tokens(stats.total.cached_input_tokens),
        full_tokens(stats.total.output_tokens),
        full_tokens(stats.total.reasoning_output_tokens),
        full_tokens(unpriced_tokens(stats)),
        model_lines,
        stats.events_seen,
        stats.files_seen
    )
}

fn fetched_at_text(ts: Option<i64>) -> String {
    let Some(ts) = ts else {
        return "n/a".into();
    };
    let Some(dt) = Local.timestamp_opt(ts, 0).single() else {
        return "n/a".into();
    };
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn make_details_html(stats: &Stats) -> String {
    let total_cost = sum_cost(&stats.by_model);
    let today_cost = sum_cost(&stats.today_by_model);
    let month_cost = sum_cost(&stats.month_by_model);
    let month_range = current_month_range_text();
    let rate = stats
        .rate_limits
        .clone()
        .and_then(|r| display_rate_limits(&r))
        .unwrap_or_default();
    let primary_title = if window_present(&rate.primary) {
        window_full(rate.primary.window_duration_mins)
    } else {
        "n/a"
    };
    let secondary_title = if window_present(&rate.secondary) {
        window_full(rate.secondary.window_duration_mins)
    } else {
        "n/a"
    };
    let config = load_config();
    let mut top: Vec<_> = stats.by_model.iter().collect();
    top.sort_by_key(|(_, usage)| -usage.total_tokens);
    let model_rows = top
        .into_iter()
        .take(8)
        .map(|(model, usage)| {
            format!(
                "<tr>\
                    <td><span class=\"model-name\">{}</span></td>\
                    <td title=\"{}\">{}</td>\
                    <td title=\"{}\">{}</td>\
                    <td title=\"{}\">{}</td>\
                    <td title=\"{}\">{}</td>\
                    <td title=\"{}\">{}</td>\
                    <td class=\"cost\">{}</td>\
                </tr>",
                html_escape(model),
                full_tokens(usage.total_tokens),
                compact_tokens(usage.total_tokens),
                full_tokens(usage.input_tokens),
                compact_tokens(usage.input_tokens),
                full_tokens(usage.cached_input_tokens),
                compact_tokens(usage.cached_input_tokens),
                full_tokens(usage.output_tokens),
                compact_tokens(usage.output_tokens),
                full_tokens(usage.reasoning_output_tokens),
                compact_tokens(usage.reasoning_output_tokens),
                dollars(cost_for_usage(model, usage))
            )
        })
        .collect::<Vec<_>>()
        .join("");
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Codex Usage</title>
<style>
:root {{
  color-scheme: dark;
  --bg: #090d12;
  --panel: #111820;
  --panel-soft: #0d131a;
  --line: #202b36;
  --line-soft: #18222d;
  --text: #eef4fb;
  --muted: #8ea0b2;
  --quiet: #627284;
  --green: #35c46a;
  --amber: #e5b454;
  --red: #e35d5d;
  --blue: #6aa8ff;
  --violet: #aa7cff;
}}
* {{ box-sizing: border-box; }}
body {{
  margin: 0;
  min-height: 100dvh;
  background:
    radial-gradient(circle at 15% 0%, rgba(106, 168, 255, 0.13), transparent 34rem),
    radial-gradient(circle at 85% 8%, rgba(170, 124, 255, 0.11), transparent 30rem),
    linear-gradient(180deg, #0b1016 0%, var(--bg) 48%, #070a0f 100%);
  color: var(--text);
  font: 14px/1.5 "SF Pro Text", -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}}
.shell {{
  width: min(1180px, calc(100vw - 32px));
  margin: 0 auto;
  padding: 34px 0 42px;
}}
.topbar {{
  display: flex;
  align-items: end;
  justify-content: space-between;
  gap: 24px;
  margin-bottom: 22px;
}}
h1 {{
  margin: 0;
  font-size: clamp(30px, 3.7vw, 46px);
  line-height: 1.04;
  letter-spacing: 0;
  font-weight: 650;
}}
.subtitle {{
  margin: 10px 0 0;
  max-width: 58ch;
  color: var(--muted);
}}
.plan {{
  border: 1px solid var(--line);
  border-radius: 14px;
  padding: 10px 14px;
  background: rgba(17, 24, 32, 0.72);
  color: #d9e6f2;
  white-space: nowrap;
}}
.grid {{
  display: grid;
  grid-template-columns: repeat(12, 1fr);
  gap: 12px;
}}
.card {{
  border: 1px solid var(--line);
  border-radius: 16px;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.035), transparent),
    rgba(17, 24, 32, 0.84);
  box-shadow: 0 18px 70px rgba(0, 0, 0, 0.22);
  overflow: hidden;
}}
.card.pad {{ padding: 18px; }}
.span-3 {{ grid-column: span 3; }}
.span-4 {{ grid-column: span 4; }}
.span-6 {{ grid-column: span 6; }}
.span-8 {{ grid-column: span 8; }}
.span-12 {{ grid-column: span 12; }}
.label {{
  color: var(--quiet);
  font-size: 12px;
  font-weight: 650;
  margin-bottom: 8px;
}}
.metric {{
  font-size: clamp(23px, 2.8vw, 32px);
  line-height: 1;
  font-weight: 650;
  letter-spacing: 0;
}}
.muted {{ color: var(--muted); }}
.small {{ font-size: 12px; color: var(--quiet); }}
.rate-head {{
  display: flex;
  justify-content: space-between;
  gap: 18px;
  align-items: start;
  margin-bottom: 14px;
}}
.rate-title {{
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 740;
}}
.dot {{
  width: 9px;
  height: 9px;
  border-radius: 999px;
  background: var(--dot);
  box-shadow: 0 0 18px var(--dot);
}}
.progress {{
  height: 8px;
  border-radius: 999px;
  background: #0a0f15;
  border: 1px solid var(--line-soft);
  overflow: hidden;
}}
.bar {{
  height: 100%;
  width: var(--value);
  background: linear-gradient(90deg, var(--dot), color-mix(in srgb, var(--dot), #ffffff 18%));
  border-radius: inherit;
}}
.rate-meta {{
  display: flex;
  justify-content: space-between;
  gap: 16px;
  margin-top: 12px;
}}
.pace {{
  margin-top: 10px;
  color: var(--muted);
  font-size: 12px;
}}
.breakdown {{
  display: grid;
  gap: 10px;
}}
.breakdown-row {{
  display: grid;
  grid-template-columns: 1fr auto;
  gap: 16px;
  padding: 10px 0;
  border-bottom: 1px solid var(--line-soft);
}}
.breakdown-row:last-child {{ border-bottom: 0; }}
.table-wrap {{ overflow: hidden; }}
table {{
  width: 100%;
  table-layout: fixed;
  border-collapse: collapse;
}}
th, td {{
  padding: 12px 10px;
  text-align: right;
  border-bottom: 1px solid var(--line-soft);
  color: #d8e4ef;
  white-space: nowrap;
}}
th {{
  color: var(--quiet);
  font-size: 11px;
  font-weight: 700;
}}
th:first-child, td:first-child {{
  width: 24%;
  text-align: left;
}}
th:not(:first-child), td:not(:first-child) {{
  width: 12.666%;
}}
tr:last-child td {{ border-bottom: 0; }}
.model-name {{
  display: inline-flex;
  align-items: center;
  max-width: 100%;
  border: 1px solid var(--line);
  border-radius: 999px;
  padding: 3px 8px;
  background: rgba(255, 255, 255, 0.025);
  font-weight: 650;
  overflow: hidden;
  text-overflow: ellipsis;
  vertical-align: middle;
}}
.cost {{ color: #cfe0ff; font-weight: 720; }}
.footer {{
  margin-top: 12px;
  color: var(--quiet);
  display: flex;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
}}
@media (max-width: 860px) {{
  .shell {{ width: min(100vw - 22px, 1180px); padding-top: 22px; }}
  .topbar {{ display: block; }}
  .plan {{ display: inline-flex; margin-top: 16px; }}
  .span-3, .span-4, .span-6, .span-8 {{ grid-column: span 12; }}
  th, td {{ padding: 10px 7px; font-size: 12px; }}
  .model-name {{ max-width: 92px; }}
}}
</style>
</head>
<body>
<main class="shell">
  <header class="topbar">
    <div>
      <h1>Codex Usage</h1>
      <p class="subtitle">Subscription usage, API-equivalent cost, cached-token pricing and reset windows from local Codex session events.</p>
    </div>
    <div class="plan">{}</div>
  </header>

  <section class="grid">
    <article class="card pad span-6" style="--dot:{}">
      <div class="rate-head">
        <div class="rate-title"><span class="dot"></span><span>{} rate limit</span></div>
        <div class="metric">{:.0}%</div>
      </div>
      <div class="progress"><div class="bar" style="--value:{:.4}%"></div></div>
      <div class="rate-meta">
        <span class="small">Resets in <strong class="muted">{}</strong></span>
        <span class="small">At <strong class="muted">{}</strong></span>
      </div>
    </article>

    <article class="card pad span-6" style="--dot:{}">
      <div class="rate-head">
        <div class="rate-title"><span class="dot"></span><span>{} rate limit</span></div>
        <div class="metric">{:.0}%</div>
      </div>
      <div class="progress"><div class="bar" style="--value:{:.4}%"></div></div>
      <div class="rate-meta">
        <span class="small">Resets in <strong class="muted">{}</strong></span>
        <span class="small">At <strong class="muted">{}</strong></span>
      </div>
    </article>

    <article class="card pad span-12" style="--dot:{}">
      <div class="rate-head">
        <div class="rate-title"><span class="dot"></span><span>Usage pace</span></div>
        <div class="metric">{}</div>
      </div>
        <div class="small">Expected <strong class="muted">{:.1}%</strong> of the {} window by now.</div>
    </article>

    <article class="card pad span-12">
      <div class="rate-head">
        <div>
          <div class="label">Party mode</div>
          <div class="metric">{}</div>
        </div>
        <div class="plan">{}</div>
      </div>
      <div class="small">Reset notifications always stay enabled. Party mode controls the fullscreen confetti overlay and can be toggled from the tray menu.</div>
    </article>

    <article class="card pad span-12">
      <div class="rate-head">
        <div>
          <div class="label">Refresh interval</div>
          <div class="metric">{}</div>
        </div>
        <div class="plan">Tray menu setting</div>
      </div>
      <div class="small">This controls how often the app re-walks local Codex session files. Lower values feel more live; higher values use less background work.</div>
    </article>

    <article class="card pad span-4">
      <div class="label">Today's cost</div>
      <div class="metric">{}</div>
      <div class="small">{} tokens</div>
    </article>
    <article class="card pad span-4">
      <div class="label">Monthly cost</div>
      <div class="metric">{}</div>
      <div class="small">{} tokens</div>
      <div class="small">{}</div>
    </article>
    <article class="card pad span-4">
      <div class="label">All-time estimate</div>
      <div class="metric">{}</div>
      <div class="small">{} tokens</div>
    </article>

    <article class="card pad span-4">
      <div class="label">Token breakdown</div>
      <div class="breakdown">
        <div class="breakdown-row"><span>Input</span><strong>{}</strong></div>
        <div class="breakdown-row"><span>Cached input</span><strong>{}</strong></div>
        <div class="breakdown-row"><span>Output</span><strong>{}</strong></div>
        <div class="breakdown-row"><span>Reasoning</span><strong>{}</strong></div>
        <div class="breakdown-row"><span>Unpriced</span><strong>{}</strong></div>
      </div>
    </article>

    <article class="card span-8">
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Model</th>
              <th>Total</th>
              <th>Input</th>
              <th>Cached</th>
              <th>Output</th>
              <th>Reasoning</th>
              <th>Cost</th>
            </tr>
          </thead>
          <tbody>{}</tbody>
        </table>
      </div>
    </article>
  </section>

  <footer class="footer">
    <span>Source: {} token_count events from {} JSONL files</span>
    <span>Costs include cached input pricing when token data is present.</span>
  </footer>
</main>
</body>
</html>"#,
        html_escape(display_plan(&rate.plan_type)),
        primary_title,
        rate_color(rate.primary.used_percent),
        rate.primary.used_percent,
        rate.primary.used_percent.clamp(0.0, 100.0),
        html_escape(&reset_text(rate.primary.resets_at)),
        html_escape(&reset_clock_text(rate.primary.resets_at)),
        secondary_title,
        rate_color(rate.secondary.used_percent),
        rate.secondary.used_percent,
        rate.secondary.used_percent.clamp(0.0, 100.0),
        html_escape(&reset_text(rate.secondary.resets_at)),
        html_escape(&reset_clock_text(rate.secondary.resets_at)),
        pace_color(&rate.primary),
        html_escape(&pace_text(&rate.primary)),
        primary_pace(&rate.primary)
            .map(|pace| pace.expected_percent)
            .unwrap_or(0.0),
        window_full(rate.primary.window_duration_mins),
        if config.party_mode { "On" } else { "Off" },
        if config.party_mode {
            "Confetti enabled"
        } else {
            "Notifications only"
        },
        duration_label(config.refresh_seconds),
        dollars(today_cost),
        full_tokens(stats.today.total_tokens),
        dollars(month_cost),
        full_tokens(stats.month.total_tokens),
        html_escape(&month_range),
        dollars(total_cost),
        full_tokens(stats.total.total_tokens),
        full_tokens(stats.total.input_tokens),
        full_tokens(stats.total.cached_input_tokens),
        full_tokens(stats.total.output_tokens),
        full_tokens(stats.total.reasoning_output_tokens),
        full_tokens(unpriced_tokens(stats)),
        model_rows,
        stats.events_seen,
        stats.files_seen
    )
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn rate_color(percent: f64) -> &'static str {
    if percent >= 85.0 {
        "#e35d5d"
    } else if percent >= 60.0 {
        "#e5b454"
    } else {
        "#35c46a"
    }
}

fn pace_color(limit: &WindowLimit) -> &'static str {
    let Some(pace) = primary_pace(limit) else {
        return "#627284";
    };
    if pace.ahead_percent >= PACE_ALERT_AHEAD_PERCENT {
        "#e35d5d"
    } else if pace.ahead_percent > 5.0 {
        "#e5b454"
    } else {
        "#35c46a"
    }
}

fn c_string(value: &str) -> CString {
    CString::new(value.replace('\0', "")).unwrap()
}

fn ensure_empty_icon_path() -> String {
    let dir = env::temp_dir().join("StatusBar-Codex-Linux-icons");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("codex-usage-empty.svg");
    if !path.exists() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" viewBox="0 0 1 1"></svg>"#;
        let _ = fs::write(&path, svg);
    }
    path.to_string_lossy().into_owned()
}

/// Build tray icon PNG. Returns icon name (no path/ext) for app_indicator_set_icon_full.
/// Does NOT lock STATE — caller must set the indicator icon.
fn ensure_label_icon(label: &str, _pct: f64) -> String {
    let ind_dir = paths::icon_dir();
    let _ = fs::create_dir_all(&ind_dir);
    // Unique name forces AppIndicator to reload each animation frame.
    let icon_name = if show_mascot() {
        format!("StatusBar-Codex-Linux-{}", mascot::icon_suffix())
    } else {
        "StatusBar-Codex-Linux".into()
    };
    let path = ind_dir.join(format!("{icon_name}.png"));

    if show_mascot() {
        let frame = mascot::current_frame_path();
        // GIF ships with opaque gray bg. Kill it (fuzz covers the ~180-220 gray ramp)
        // so only the robot draws on the tray's real background; white label floats right.
        let _ = Command::new("convert")
            .arg("-size").arg("110x34").arg("xc:none")
            .arg("(").arg(&frame).arg("-resize").arg("x30")
            .arg("-fuzz").arg("12%")
            .arg("-transparent").arg("gray(180,180,180)")
            .arg(")")
            .arg("-geometry").arg("+3+2").arg("-compose").arg("over").arg("-composite")
            .arg("-fill").arg("white")
            .arg("-font").arg("DejaVu-Sans-Bold")
            .arg("-pointsize").arg("15")
            .arg("-annotate").arg(format!("+52+{}", 25)) // vertical center
            .arg(label.replace('%', "%%"))
            .arg(&path)
            .output();
    } else {
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="28" viewBox="0 0 200 28">
  <text x="100" y="20" text-anchor="middle" font-family="SF Mono, JetBrains Mono, monospace" font-size="15" font-weight="700" fill="#ffffff">{text}</text>
</svg>"##,
            text = html_escape(label),
        );
        let svg_path = ind_dir.join(format!("{icon_name}.svg"));
        let _ = fs::write(&svg_path, svg);
        let _ = Command::new("convert")
            .args([
                "-background",
                "none",
                "-size",
                "200x28",
                &svg_path.to_string_lossy(),
                &path.to_string_lossy(),
            ])
            .output();
    }

    let _ = fs::create_dir_all(ind_dir.join("apps"));
    let _ = fs::copy(&path, ind_dir.join("apps").join(format!("{icon_name}.png")));
    let _ = fs::copy(&path, ind_dir.join("StatusBar-Codex-Linux.png"));

    let themed_dir = home_icon_dir();
    let _ = fs::create_dir_all(&themed_dir);
    let _ = fs::copy(&path, themed_dir.join("StatusBar-Codex-Linux.png"));

    icon_name
}

unsafe fn set_tray_icon(indicator: *mut AppIndicator, icon_name: &str) {
    let name = c_string(icon_name);
    let desc = c_string("Codex usage");
    unsafe {
        app_indicator_set_icon_full(indicator, name.as_ptr(), desc.as_ptr());
    }
}

fn home_icon_dir() -> PathBuf {
    if let Some(home) = env::var_os("HOME") {
        PathBuf::from(home).join(".local/share/icons/hicolor/scalable/apps")
    } else {
        PathBuf::from("/tmp/StatusBar-Codex-Linux-icons")
    }
}

unsafe fn set_markup(label: *mut GtkWidget, markup: &str) {
    let markup = c_string(markup);
    unsafe { gtk_label_set_markup(label, markup.as_ptr()) };
}

unsafe extern "C" fn on_refresh(_widget: *mut GtkWidget, _data: *mut c_void) {
    update_state(true);
}

unsafe extern "C" fn on_details(_widget: *mut GtkWidget, _data: *mut c_void) {
    let stats = collect_stats();
    let details = details_html_path();
    let _ = fs::write(&details, make_details_html(&stats));
    let _ = Command::new("xdg-open").arg(details).spawn();
}

unsafe extern "C" fn on_toggle_party_mode(_widget: *mut GtkWidget, _data: *mut c_void) {
    set_party_mode(!party_mode_enabled());
    update_state(true);
}

unsafe extern "C" fn on_toggle_mascot(_widget: *mut GtkWidget, _data: *mut c_void) {
    set_show_mascot(!show_mascot());
    update_state(true);
}

unsafe extern "C" fn on_mascot_anim(_data: *mut c_void) -> c_int {
    if !show_mascot() {
        return 1;
    }
    mascot::advance_frame();
    if let Some(state) = STATE.get() {
        let state = state.lock().unwrap();
        if let Some(snap) = state.last_render.as_ref() {
            let label = snap.svg_label.clone();
            let indicator = state.indicator;
            drop(state);
            let icon_name = ensure_label_icon(&label, 0.0);
            set_tray_icon(indicator, &icon_name);
        }
    }
    1
}

unsafe extern "C" fn on_refresh_5s(_widget: *mut GtkWidget, _data: *mut c_void) {
    set_refresh_seconds(5);
    update_state(true);
}

unsafe extern "C" fn on_refresh_15s(_widget: *mut GtkWidget, _data: *mut c_void) {
    set_refresh_seconds(15);
    update_state(true);
}

unsafe extern "C" fn on_refresh_30s(_widget: *mut GtkWidget, _data: *mut c_void) {
    set_refresh_seconds(30);
    update_state(true);
}

unsafe extern "C" fn on_refresh_60s(_widget: *mut GtkWidget, _data: *mut c_void) {
    set_refresh_seconds(60);
    update_state(true);
}

unsafe extern "C" fn on_refresh_300s(_widget: *mut GtkWidget, _data: *mut c_void) {
    set_refresh_seconds(300);
    update_state(true);
}

unsafe extern "C" fn on_quit(_widget: *mut GtkWidget, _data: *mut c_void) {
    unsafe { gtk_main_quit() };
}

unsafe extern "C" fn on_timer(_data: *mut c_void) -> c_int {
    update_state(false);
    1
}

unsafe extern "C" fn quit_timer(_data: *mut c_void) -> c_int {
    unsafe { gtk_main_quit() };
    0
}

unsafe fn make_window_transparent(window: *mut GtkWidget) {
    unsafe {
        gtk_widget_set_app_paintable(window, 1);
        let screen = gtk_widget_get_screen(window);
        if !screen.is_null() {
            let visual = gdk_screen_get_rgba_visual(screen);
            if !visual.is_null() {
                gtk_widget_set_visual(window, visual);
            }
            let provider = gtk_css_provider_new();
            let css = c_string(
                "window, label { background: transparent; background-color: transparent; }\
                 label { color: #f8fafc; text-shadow: 0 2px 8px rgba(0,0,0,0.85); }",
            );
            gtk_css_provider_load_from_data(provider, css.as_ptr(), -1, ptr::null_mut());
            gtk_style_context_add_provider_for_screen(screen, provider, 600);
        }
    }
}

unsafe fn make_window_click_through(window: *mut GtkWidget) {
    unsafe {
        let region = cairo_region_create();
        if !region.is_null() {
            gtk_widget_input_shape_combine_region(window, region);
            cairo_region_destroy(region);
        }
    }
}

struct Particle {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    size: f64,
    spin: f64,
    angle: f64,
    color: (f64, f64, f64),
}

struct ConfettiState {
    window: *mut GtkWidget,
    canvas: *mut GtkWidget,
    emoji_label: *mut GtkWidget,
    particles: Vec<Particle>,
    frames_left: i32,
    total_frames: i32,
    width: f64,
    height: f64,
}

fn next_rand(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    ((*seed >> 32) as f64) / (u32::MAX as f64)
}

fn make_particles(count: usize, width: f64, height: f64, big: bool) -> Vec<Particle> {
    let colors = [
        (0.98, 0.32, 0.32),
        (0.20, 0.77, 0.42),
        (0.28, 0.56, 1.00),
        (0.95, 0.78, 0.22),
        (0.72, 0.42, 1.00),
        (1.00, 0.48, 0.18),
    ];
    let mut seed = if big { 0xfeed_cafe } else { 0xdeca_fbad };
    let mut particles = Vec::with_capacity(count);
    for i in 0..count {
        let x = next_rand(&mut seed) * width;
        let y = -height * next_rand(&mut seed);
        let spread = if big { 4.8 } else { 3.0 };
        particles.push(Particle {
            x,
            y,
            vx: (next_rand(&mut seed) - 0.5) * spread,
            vy: 2.2 + next_rand(&mut seed) * if big { 6.2 } else { 4.2 },
            size: 5.0 + next_rand(&mut seed) * if big { 12.0 } else { 8.0 },
            spin: (next_rand(&mut seed) - 0.5) * 0.28,
            angle: next_rand(&mut seed) * std::f64::consts::TAU,
            color: colors[i % colors.len()],
        });
    }
    particles
}

unsafe extern "C" fn draw_confetti(
    widget: *mut GtkWidget,
    cr: *mut Cairo,
    data: *mut c_void,
) -> c_int {
    let state = unsafe { &mut *(data as *mut ConfettiState) };
    let width = unsafe { gtk_widget_get_allocated_width(widget) }.max(1) as f64;
    let height = unsafe { gtk_widget_get_allocated_height(widget) }.max(1) as f64;
    let elapsed = (state.total_frames - state.frames_left).max(0) as f64;
    let fade_in = (elapsed / 28.0).clamp(0.0, 1.0);
    let fade_out = (state.frames_left as f64 / 45.0).clamp(0.0, 1.0);
    let alpha = fade_in.min(fade_out);
    unsafe {
        cairo_set_operator(cr, 1);
        cairo_set_source_rgba(cr, 0.0, 0.0, 0.0, 0.0);
        cairo_paint(cr);
        cairo_set_operator(cr, 2);

        for particle in &state.particles {
            cairo_save(cr);
            cairo_translate(
                cr,
                particle.x * width / state.width,
                particle.y * height / state.height,
            );
            cairo_rotate(cr, particle.angle);
            cairo_set_source_rgba(
                cr,
                particle.color.0,
                particle.color.1,
                particle.color.2,
                0.92 * alpha,
            );
            if particle.size > 11.0 {
                cairo_arc(
                    cr,
                    0.0,
                    0.0,
                    particle.size * 0.45,
                    0.0,
                    std::f64::consts::TAU,
                );
            } else {
                cairo_rectangle(
                    cr,
                    -particle.size * 0.55,
                    -particle.size * 0.30,
                    particle.size * 1.10,
                    particle.size * 0.60,
                );
            }
            cairo_fill(cr);
            cairo_restore(cr);
        }
    }
    0
}

unsafe extern "C" fn tick_confetti(data: *mut c_void) -> c_int {
    let state = unsafe { &mut *(data as *mut ConfettiState) };
    state.frames_left -= 1;
    if state.frames_left <= 0 {
        unsafe { gtk_widget_destroy(state.window) };
        drop(unsafe { Box::from_raw(data as *mut ConfettiState) });
        return 0;
    }
    for particle in &mut state.particles {
        particle.x += particle.vx;
        particle.y += particle.vy;
        particle.vy += 0.035;
        particle.angle += particle.spin;
        if particle.y > state.height + 40.0 {
            particle.y = -20.0;
        }
        if particle.x < -40.0 {
            particle.x = state.width + 40.0;
        } else if particle.x > state.width + 40.0 {
            particle.x = -40.0;
        }
    }
    let elapsed = (state.total_frames - state.frames_left).max(0) as f64;
    let fade_in = (elapsed / 28.0).clamp(0.0, 1.0);
    let fade_out = (state.frames_left as f64 / 45.0).clamp(0.0, 1.0);
    unsafe { gtk_widget_set_opacity(state.emoji_label, fade_in.min(fade_out)) };
    unsafe { gtk_widget_queue_draw(state.canvas) };
    1
}

fn overlay_markup(message: &str, big: bool) -> String {
    let body = html_escape(if big {
        "THE WEEKLY RATE LIMIT HAS BEEN RESET!"
    } else if message.contains("5 hour") {
        "The 5 hour rate limit has been reset!"
    } else {
        message
    });
    if big {
        format!(
            "<span font_desc=\"72\">🎉 🎊 🥳 ✨ 🎉</span>\n<span font_desc=\"24\" weight=\"bold\">{body}</span>"
        )
    } else {
        format!(
            "<span font_desc=\"58\">🎉 🎊 ✨</span>\n<span font_desc=\"20\" weight=\"bold\">{body}</span>"
        )
    }
}

fn show_confetti(message: &str, big: bool) {
    unsafe {
        let window = gtk_window_new(0);
        let width = 1920.0;
        let height = 1080.0;
        let frames = 5 * 60;
        gtk_layer_init_for_window(window);
        gtk_layer_set_namespace(window, c_string("codex-usage-party").as_ptr());
        gtk_layer_set_layer(window, 3);
        gtk_layer_set_anchor(window, 0, 1);
        gtk_layer_set_anchor(window, 1, 1);
        gtk_layer_set_anchor(window, 2, 1);
        gtk_layer_set_anchor(window, 3, 1);
        gtk_layer_set_margin(window, 2, 0);
        gtk_layer_set_exclusive_zone(window, -1);
        gtk_layer_set_keyboard_mode(window, 0);
        gtk_window_set_title(window, c_string("Codex Usage Party").as_ptr());
        gtk_window_set_default_size(window, width as c_int, height as c_int);
        gtk_window_set_decorated(window, 0);
        gtk_window_set_keep_above(window, 1);
        gtk_widget_set_size_request(window, width as c_int, height as c_int);
        make_window_transparent(window);
        make_window_click_through(window);

        let overlay = gtk_overlay_new();
        let canvas = gtk_drawing_area_new();
        gtk_widget_set_size_request(canvas, width as c_int, height as c_int);
        gtk_container_add(overlay, canvas);

        let emoji_label = gtk_label_new(ptr::null());
        gtk_label_set_markup(
            emoji_label,
            c_string(&overlay_markup(message, big)).as_ptr(),
        );
        gtk_label_set_xalign(emoji_label, 0.5);
        gtk_widget_set_halign(emoji_label, 3);
        gtk_widget_set_valign(emoji_label, 3);
        gtk_widget_set_opacity(emoji_label, 0.0);
        gtk_overlay_add_overlay(overlay, emoji_label);

        let state = Box::new(ConfettiState {
            window,
            canvas,
            emoji_label,
            particles: make_particles(if big { 260 } else { 150 }, width, height, big),
            frames_left: frames,
            total_frames: frames,
            width,
            height,
        });
        let state_ptr = Box::into_raw(state);
        g_signal_connect_data(
            canvas as *mut c_void,
            c_string("draw").as_ptr(),
            draw_confetti as *mut c_void,
            state_ptr as *mut c_void,
            ptr::null_mut(),
            0,
        );
        gtk_container_add(window, overlay);
        gtk_widget_show_all(window);
        g_timeout_add(16, Some(tick_confetti), state_ptr as *mut c_void);
    }
}

fn party_overlay_allowed() -> bool {
    let now = Utc::now().timestamp();
    let last_party = LAST_PARTY_OVERLAY_AT.get_or_init(|| Mutex::new(None));
    let mut last_party = last_party.lock().unwrap();
    if last_party.is_some_and(|last| now - last < PARTY_OVERLAY_COOLDOWN_SECONDS) {
        return false;
    }
    *last_party = Some(now);
    true
}

fn send_reset_notification(body: &str, party: bool) {
    let _ = Command::new("notify-send")
        .arg("Codex Usage")
        .arg(body)
        .spawn();
    mascot::notify_success();
    if party_mode_enabled() && party_overlay_allowed() {
        show_confetti(body, party);
    }
}

fn send_plain_notification(body: &str) {
    let _ = Command::new("notify-send")
        .arg("Codex Usage")
        .arg(body)
        .spawn();
}

fn maybe_notify_primary_reset(state: &mut AppState, rate: &RateLimits) {
    // Live account/rateLimits/read can drift resets_at slightly each poll.
    // Never notify on tiny moves — only a real new 5h window.
    let current_reset = rate.primary.resets_at;
    if state.seen_primary_window {
        if let (Some(previous), Some(current)) = (state.last_primary_resets_at, current_reset) {
            let jumped = current >= previous + (PRIMARY_WINDOW_SECONDS - 30 * 60);
            let already = state.last_primary_reset_notified_at == Some(current);
            if jumped && !already {
                state.last_primary_reset_notified_at = Some(current);
                // ponytail: notifications off by default — live drift used to spam every refresh.
                debug_log(&format!(
                    "5h window advanced previous={previous} current={current} (notify disabled)"
                ));
            }
        }
    }
    state.seen_primary_window = true;
    state.last_primary_resets_at = current_reset;
}

fn maybe_notify_fast_pace(state: &mut AppState, rate: &RateLimits) {
    // Notifications disabled (live refresh made these noisy).
    state.pace_alert_window = rate.primary.resets_at;
    state.pace_alert_active = false;
}

fn maybe_notify_secondary_reset(state: &mut AppState, rate: &RateLimits) {
    // Same as 5h: track window only, no desktop spam.
    state.seen_secondary_window = true;
    state.last_secondary_resets_at = rate.secondary.resets_at;
}

fn make_render_snapshot(stats: &Stats) -> RenderSnapshot {
    let rate = stats
        .rate_limits
        .as_ref()
        .and_then(display_rate_limits);
    let primary = rate
        .as_ref()
        .map(|r| r.primary.clone())
        .unwrap_or_default();
    let secondary = rate
        .as_ref()
        .map(|r| r.secondary.clone())
        .unwrap_or_default();
    let plan = rate
        .as_ref()
        .map(|r| display_plan(&r.plan_type).to_string())
        .unwrap_or_else(|| "n/a".into());
    let present_windows: Vec<&WindowLimit> = if let Some(rate) = &rate {
        let mut v = Vec::new();
        if window_present(&rate.primary) {
            v.push(&rate.primary);
        }
        if window_present(&rate.secondary) {
            v.push(&rate.secondary);
        }
        v
    } else {
        Vec::new()
    };
    let tray_label = if present_windows.is_empty() {
        "Codex status unavailable".to_string()
    } else {
        present_windows
            .iter()
            .map(|w| format!("{} {:.0}%", window_short(w.window_duration_mins), w.used_percent))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let title = if present_windows.is_empty() {
        "Codex status unavailable — open Codex CLI or check auth".to_string()
    } else {
        format!("Codex | {} | {}", tray_label, plan)
    };
    RenderSnapshot {
        primary_header_markup: if window_present(&primary) {
            format!(
                "<b>{} limit</b>  |  reset in {} ({})",
                window_full(primary.window_duration_mins),
                reset_text(primary.resets_at),
                reset_clock_text(primary.resets_at)
            )
        } else {
            "<b>limit unavailable</b>".into()
        },
        limit_markup: if window_present(&primary) {
            format!(
                "{}  <b><span color='{}'>{:.0}%</span> used</b>",
                usage_bar(primary.used_percent),
                rate_color(primary.used_percent),
                primary.used_percent
            )
        } else {
            "no recent Codex/ChatGPT status".into()
        },
        weekly_header_markup: if window_present(&secondary) {
            format!(
                "<b>{} limit</b>  |  reset in {} ({})",
                window_full(secondary.window_duration_mins),
                reset_text(secondary.resets_at),
                reset_clock_text(secondary.resets_at)
            )
        } else {
            "<b>limit unavailable</b>".into()
        },
        weekly_markup: if window_present(&secondary) {
            format!(
                "{}  <b><span color='{}'>{:.0}%</span> used</b>",
                usage_bar(secondary.used_percent),
                rate_color(secondary.used_percent),
                secondary.used_percent
            )
        } else {
            "no recent Codex/ChatGPT status".into()
        },
        pace_markup: if window_present(&primary) {
            pace_delta_markup(&primary)
        } else {
            "<b>Pace:</b>  n/a".into()
        },
        plan_markup: format!("<b>Plan:</b>  {}", plan),
        auth_markup: format!("<b>Auth:</b>  {}", stats.auth_status),
        updated_markup: format!(
            "<b>Updated:</b>  {}",
            fetched_at_text(stats.status_fetched_at)
        ),
        source_markup: format!("<b>Source:</b>  {}", stats.status_source.label()),
        party_mode_markup: party_mode_markup(),
        mascot_markup: mascot_markup(),
        refresh_interval_markup: refresh_interval_markup(),
        tray_label: tray_label.clone(),
        svg_label: tray_label.clone(),
        title,
    }
}

fn update_state(force: bool) {
    if let Some(state) = STATE.get() {
        let mut state = state.lock().unwrap();
        let now = Utc::now().timestamp();
        let refresh_due = state
            .last_refresh_at
            .is_none_or(|last| now - last >= refresh_seconds() as i64);
        if !force && !refresh_due {
            return;
        }
        state.last_refresh_at = Some(now);
        let stats = collect_stats();
        let primary_pct = stats
            .rate_limits
            .as_ref()
            .map(|r| r.primary.used_percent)
            .unwrap_or(0.0);
        if let Some(rate) = stats.rate_limits.clone() {
            maybe_notify_primary_reset(&mut state, &rate);
            maybe_notify_fast_pace(&mut state, &rate);
            maybe_notify_secondary_reset(&mut state, &rate);
        }
        let snapshot = make_render_snapshot(&stats);
        if state.last_render.as_ref() == Some(&snapshot) {
            return;
        }
        unsafe {
            set_markup(state.primary_header_label, &snapshot.primary_header_markup);
            set_markup(state.limit_label, &snapshot.limit_markup);
            set_markup(state.weekly_header_label, &snapshot.weekly_header_markup);
            set_markup(state.weekly_label, &snapshot.weekly_markup);
            set_markup(state.pace_label, &snapshot.pace_markup);
            set_markup(state.plan_label, &snapshot.plan_markup);
            set_markup(state.auth_label, &snapshot.auth_markup);
            set_markup(state.updated_label, &snapshot.updated_markup);
            set_markup(state.source_label, &snapshot.source_markup);
            set_markup(state.party_mode_label, &snapshot.party_mode_markup);
            set_markup(state.mascot_label, &snapshot.mascot_markup);
            set_markup(
                state.refresh_interval_label,
                &snapshot.refresh_interval_markup,
            );
            let tray_label = c_string(&snapshot.tray_label);
            let guide = c_string("W 100%");
            app_indicator_set_label(state.indicator, tray_label.as_ptr(), guide.as_ptr());
            let title = c_string(&snapshot.title);
            app_indicator_set_title(state.indicator, title.as_ptr());
            // Regenerate the themed icon file so GTK picks it up on theme refresh
            mascot::tick(primary_pct);
            let icon_name = ensure_label_icon(&snapshot.svg_label, primary_pct);
            set_tray_icon(state.indicator, &icon_name);
        }
        state.last_render = Some(snapshot);
    }
}

unsafe fn menu_item(label: &str, sensitive: bool) -> *mut GtkWidget {
    let label = c_string(label);
    let item = unsafe { gtk_menu_item_new_with_label(label.as_ptr()) };
    unsafe { gtk_widget_set_sensitive(item, if sensitive { 1 } else { 0 }) };
    item
}

unsafe fn markup_menu_item(markup: &str) -> (*mut GtkWidget, *mut GtkWidget) {
    let item = unsafe { gtk_menu_item_new() };
    let label = unsafe { gtk_label_new(ptr::null()) };
    unsafe {
        gtk_label_set_xalign(label, 0.0);
        set_markup(label, markup);
        gtk_container_add(item, label);
        gtk_widget_set_sensitive(item, 1);
    }
    (item, label)
}

unsafe fn connect_activate(
    item: *mut GtkWidget,
    callback: unsafe extern "C" fn(*mut GtkWidget, *mut c_void),
) {
    let signal = c_string("activate");
    unsafe {
        g_signal_connect_data(
            item as *mut c_void,
            signal.as_ptr(),
            callback as *mut c_void,
            ptr::null_mut(),
            ptr::null_mut(),
            0,
        );
    }
}

fn main() {
    if std::env::args().any(|arg| arg == "--once") {
        let stats = collect_stats();
        println!("{}", make_details_text(&stats));
        return;
    }
    // Single-instance guard: write PID to a runtime lock file and exit if another live instance exists.
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let lock_path = std::path::Path::new(&runtime_dir).join("StatusBar-Codex-Linux.pid");
        if lock_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&lock_path) {
                if let Ok(pid) = content.trim().parse::<i32>() {
                    if std::path::Path::new(&format!("/proc/{}", pid)).exists() {
                        eprintln!("StatusBar-Codex-Linux: another instance (pid {}) is running; exiting", pid);
                        return;
                    }
                }
            }
        }
        let _ = std::fs::write(&lock_path, format!("{}", std::process::id()));
    }
    if std::env::args().any(|arg| arg == "--html") {
        let stats = collect_stats();
        println!("{}", make_details_html(&stats));
        return;
    }
    if std::env::args().any(|arg| arg == "--test-5h-reset") {
        unsafe {
            gtk_init(ptr::null_mut(), ptr::null_mut());
            send_reset_notification("The 5 hour rate limit has been reset! 🎉", false);
            g_timeout_add_seconds(12, Some(quit_timer), ptr::null_mut());
            gtk_main();
        }
        return;
    }
    if std::env::args().any(|arg| arg == "--test-weekly-reset") {
        unsafe {
            gtk_init(ptr::null_mut(), ptr::null_mut());
            send_reset_notification("THE WEEKLY RATE LIMIT HAS BEEN RESET! 🎉🎊🥳✨", true);
            g_timeout_add_seconds(12, Some(quit_timer), ptr::null_mut());
            gtk_main();
        }
        return;
    }
    if std::env::args().any(|arg| arg == "--test-pace-alert") {
        send_plain_notification(
            "Slow down, cowboy! 🤠 You are using up your rate limit FAST. Watch out! 🐬",
        );
        return;
    }

        unsafe {
            gtk_init(ptr::null_mut(), ptr::null_mut());
            let indicator = app_indicator_new(
                c_string("StatusBar-Codex-Linux").as_ptr(),
                c_string("utilities-terminal-symbolic").as_ptr(),
                0,
            );
            app_indicator_set_status(indicator, 1);
            let icon_theme_path = c_string(&paths::icon_dir().to_string_lossy());
            app_indicator_set_icon_theme_path(indicator, icon_theme_path.as_ptr());
            let icon_desc = c_string("Codex usage");
            app_indicator_set_icon_full(indicator, c_string("StatusBar-Codex-Linux").as_ptr(), icon_desc.as_ptr());
        let menu = gtk_menu_new();
        let (brand_header, _brand_header_label) =
            markup_menu_item("<span size='larger' weight='bold' color='#16b8a6'>◆ Codex Usage Tray</span>");
        gtk_widget_set_sensitive(brand_header, 0);
        let (rate_header, _rate_header_label) = markup_menu_item("<b>Codex / ChatGPT status</b>");
        let (primary_header, primary_header_label) = markup_menu_item("<b>5h limit</b>");
        let (limit_item, limit_label) = markup_menu_item("loading...");
        let (weekly_header, weekly_header_label) = markup_menu_item("<b>Weekly limit</b>");
        let (weekly_item, weekly_label) = markup_menu_item("loading...");
        let (pace_item, pace_label) = markup_menu_item("<b>Pace:</b>  loading...");
        let (plan_item, plan_label) = markup_menu_item("<b>Plan:</b>  loading...");
        let (auth_item, auth_label) = markup_menu_item("<b>Auth:</b>  loading...");
        let (updated_item, updated_label) = markup_menu_item("<b>Updated:</b>  loading...");
        let (source_item, source_label) = markup_menu_item("<b>Source:</b>  loading...");
        gtk_widget_set_sensitive(rate_header, 0);
        gtk_widget_set_sensitive(primary_header, 0);
        gtk_widget_set_sensitive(weekly_header, 0);
        let settings = menu_item("Refresh interval", true);
        let settings_menu = gtk_menu_new();
        let (party_mode_item, party_mode_label) = markup_menu_item(&party_mode_markup());
        let (mascot_item, mascot_label) = markup_menu_item(&mascot_markup());
        let (refresh_interval_item, refresh_interval_label) =
            markup_menu_item(&refresh_interval_markup());
        let refresh_5s = menu_item("Every 5 seconds", true);
        let refresh_15s = menu_item("Every 15 seconds", true);
        let refresh_30s = menu_item("Every 30 seconds", true);
        let refresh_60s = menu_item("Every 1 minute", true);
        let refresh_300s = menu_item("Every 5 minutes", true);
        let details = menu_item("Details", true);
        let refresh = menu_item("Refresh", true);
        let quit = menu_item("Quit", true);
        for item in [
            brand_header,
            gtk_separator_menu_item_new(),
            rate_header,
            primary_header,
            limit_item,
            weekly_header,
            weekly_item,
            gtk_separator_menu_item_new(),
            pace_item,
            plan_item,
            auth_item,
            updated_item,
            source_item,
            gtk_separator_menu_item_new(),
            settings,
            details,
            gtk_separator_menu_item_new(),
            refresh,
            quit,
        ] {
            gtk_menu_shell_append(menu, item);
        }
        for item in [
            party_mode_item,
            mascot_item,
            gtk_separator_menu_item_new(),
            refresh_interval_item,
            refresh_5s,
            refresh_15s,
            refresh_30s,
            refresh_60s,
            refresh_300s,
        ] {
            gtk_menu_shell_append(settings_menu, item);
        }
        gtk_menu_item_set_submenu(settings, settings_menu);
        connect_activate(party_mode_item, on_toggle_party_mode);
        connect_activate(mascot_item, on_toggle_mascot);
        connect_activate(refresh_5s, on_refresh_5s);
        connect_activate(refresh_15s, on_refresh_15s);
        connect_activate(refresh_30s, on_refresh_30s);
        connect_activate(refresh_60s, on_refresh_60s);
        connect_activate(refresh_300s, on_refresh_300s);
        connect_activate(details, on_details);
        connect_activate(refresh, on_refresh);
        connect_activate(quit, on_quit);
        gtk_widget_show_all(menu);
        app_indicator_set_menu(indicator, menu as *mut GtkMenu);
        STATE
            .set(Mutex::new(AppState {
                indicator,
                primary_header_label,
                limit_label,
                weekly_header_label,
                weekly_label,
                pace_label,
                plan_label,
                auth_label,
                updated_label,
                source_label,
                party_mode_label,
                mascot_label,
                refresh_interval_label,
                last_render: None,
                last_refresh_at: None,
                seen_primary_window: false,
                last_primary_resets_at: None,
                last_primary_reset_notified_at: None,
                pace_alert_active: false,
                pace_alert_window: None,
                seen_secondary_window: false,
                last_secondary_resets_at: None,
            }))
            .ok();
        update_state(true);
        g_timeout_add_seconds(MIN_REFRESH_SECONDS, Some(on_timer), ptr::null_mut());
        // ~8 fps tray mascot animation
        g_timeout_add(125, Some(on_mascot_anim), ptr::null_mut());
        gtk_main();
    }
}
