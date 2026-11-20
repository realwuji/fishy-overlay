//! Persistent application settings.
//!
//! The TypeScript `Settings` type (`src/shared/types.ts`) is the full schema
//! shared with both web windows. Rust only needs a handful of sections
//! (`site`, `engine`, `capture`, `hotkeys`, `acceptedFairPlay`), so those are
//! strongly typed here and every other field (arrows, labels, evalBar, hud,
//! theme, `version`, anything added later) is carried through untouched in
//! `extra` via `#[serde(flatten)]`. That keeps the round trip lossless while
//! the UI owners evolve their sections without touching Rust.
//!
//! Persistence is a pretty-printed JSON file in the app config directory,
//! written atomically (temp file + rename). A corrupt file never prevents the
//! app from starting: it is moved aside to `settings.json.bak` and defaults are
//! used. Values read from disk (or from the UI) are validated and clamped.

use fishy_vision::{OrientationMode, Site};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub const SETTINGS_FILE: &str = "settings.json";

pub const FPS_MIN: u32 = 1;
pub const FPS_MAX: u32 = 30;
pub const MULTI_PV_MIN: u32 = 1;
pub const MULTI_PV_MAX: u32 = 8;
pub const HASH_MB_MIN: u32 = 1;
pub const HASH_MB_MAX: u32 = 16_384;
pub const DEPTH_MIN: u32 = 1;
pub const DEPTH_MAX: u32 = 60;
pub const MOVE_TIME_MIN_MS: u32 = 10;
pub const MOVE_TIME_MAX_MS: u32 = 600_000;
pub const SKILL_MAX: u32 = 20;
pub const MIN_DISPLAY_DEPTH_MAX: u32 = 40;
pub const MIN_DISPLAY_DEPTH_DEFAULT: u32 = 12;
pub const ARROWS_MIN: u64 = 1;
pub const ARROWS_MAX: u64 = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineSettings {
    pub threads: u32,
    pub hash_mb: u32,
    pub multi_pv: u32,
    pub depth_limit: Option<u32>,
    pub move_time_ms: Option<u32>,
    pub skill_level: Option<u32>,
    /// Fishy picks threads and hash for this machine; the manual numbers are ignored.
    /// Settings saved before this field existed behave as `true`.
    #[serde(default = "default_auto")]
    pub auto: bool,
    /// Depth a position must reach before its arrows are shown (0 = immediately).
    #[serde(default = "default_min_display_depth")]
    pub min_display_depth: u32,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_auto() -> bool {
    true
}

fn default_min_display_depth() -> u32 {
    MIN_DISPLAY_DEPTH_DEFAULT
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSettings {
    pub fps: u32,
    pub target_id: Option<String>,
    /// Which colour is at the bottom of the screen: `auto` (default, detected), `white` or
    /// `black` (final). Absent from the saved file while `auto`; unknown values mean `auto`.
    #[serde(
        default,
        deserialize_with = "lenient_orientation",
        skip_serializing_if = "is_auto"
    )]
    pub orientation: OrientationMode,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn is_auto(m: &OrientationMode) -> bool {
    *m == OrientationMode::Auto
}

fn lenient_orientation<'de, D: Deserializer<'de>>(d: D) -> Result<OrientationMode, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_str()
        .and_then(OrientationMode::parse)
        .unwrap_or_default())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeySettings {
    pub toggle_overlay: String,
    pub toggle_hud: String,
    pub cycle_arrows: String,
    pub recalibrate: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub accepted_fair_play: bool,
    pub site: Site,
    pub engine: EngineSettings,
    pub capture: CaptureSettings,
    pub hotkeys: HotkeySettings,
    /// Every other field of the TS `Settings` (version, arrows, labels, ...).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Mirror of `DEFAULT_SETTINGS` in `src/shared/defaults.ts`. Keep in sync.
pub fn default_json() -> Value {
    let class = |label: &str, color: &str, icon: &str| json!({ "enabled": true, "label": label, "color": color, "icon": icon });
    let tag = |label: &str, color: &str, icon: &str, enabled: bool| json!({ "enabled": enabled, "label": label, "color": color, "icon": icon });
    json!({
        "version": 1,
        "acceptedFairPlay": false,
        "site": "chessCom",
        "arrows": {
            "show": true, "count": 3, "style": "classic", "curveKnightMoves": true,
            "opacity": 0.85, "widthScale": 1, "glow": true, "animate": true,
            "colorMode": "rank",
            "rankColors": ["#22d3b0", "#3b82ff", "#a78bfa", "#f59e0b", "#94a3b8"],
            "forSide": "sideToMove", "minClass": null
        },
        "labels": {
            "show": true, "placement": "cornerBadge", "style": "iconText", "size": 1,
            "showTags": true, "maxTags": 2,
            "classes": {
                "brilliant": class("Brilliant", "#1bbfb0", "!!"),
                "great": class("Great", "#5b8bb9", "!"),
                "best": class("Best", "#6fbf4a", "\u{2605}"),
                "excellent": class("Excellent", "#8cc152", "\u{1F44D}"),
                "good": class("Good", "#9bb26b", "\u{2713}"),
                "inaccuracy": class("Inaccuracy", "#f7c631", "?!"),
                "mistake": class("Mistake", "#e58f2a", "?"),
                "blunder": class("Blunder", "#ca3431", "??")
            },
            "tags": {
                "fork": tag("Fork", "#b15cff", "\u{2442}", true),
                "pin": tag("Pin", "#ff7ab6", "\u{1F4CC}", true),
                "skewer": tag("Skewer", "#ff9f43", "\u{27FF}", true),
                "discovered": tag("Discovered", "#4cc9f0", "\u{25D0}", true),
                "hanging": tag("Hanging", "#ff5d73", "\u{2693}", true),
                "check": tag("Check", "#ffd166", "+", true),
                "mate": tag("Mate", "#ef476f", "#", true),
                "capture": tag("Capture", "#8d99ae", "\u{00D7}", false),
                "promotion": tag("Promotion", "#06d6a0", "\u{265B}", true),
                "sacrifice": tag("Sacrifice", "#f78c6b", "\u{2694}", true),
                "castle": tag("Castle", "#8d99ae", "\u{265C}", false),
                "enPassant": tag("En passant", "#8d99ae", "e.p.", false)
            }
        },
        "evalBar": {
            "show": true, "position": "left", "width": 14, "showNumber": true,
            "style": "gradient", "animate": true,
            "whiteColor": "#f4f8fc", "blackColor": "#101a26"
        },
        "hud": {
            "show": true, "corner": "br", "lines": 3, "showPv": true,
            "showExplanation": true, "showDepth": true, "showWinPct": true,
            "opacity": 0.94, "scale": 1, "compact": false
        },
        "engine": {
            "threads": 2, "hashMb": 128, "multiPv": 3,
            "depthLimit": null, "moveTimeMs": null, "skillLevel": null,
            "auto": true, "minDisplayDepth": 12
        },
        "capture": { "fps": 8, "targetId": null },
        "theme": { "mode": "dark", "accent": "#22d3b0", "uiScale": 1, "reduceMotion": false },
        "hotkeys": {
            "toggleOverlay": "Alt+Space", "toggleHud": "Alt+H",
            "cycleArrows": "Alt+A", "recalibrate": "Alt+R"
        }
    })
}

impl Default for Settings {
    fn default() -> Self {
        let mut s: Settings =
            serde_json::from_value(default_json()).expect("default settings are well-formed");
        s.validate();
        s
    }
}

/// Number of logical CPUs (at least 1).
pub fn cpu_count() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
}

fn prev_power_of_two(n: u32) -> u32 {
    if n == 0 {
        1
    } else {
        1 << (31 - n.leading_zeros())
    }
}

impl Settings {
    /// Clamp every Rust-visible value into its supported range.
    pub fn validate(&mut self) {
        self.validate_with_cpus(cpu_count());
    }

    pub fn validate_with_cpus(&mut self, cpus: u32) {
        let e = &mut self.engine;
        e.threads = e.threads.clamp(1, cpus.max(1));
        e.hash_mb = prev_power_of_two(e.hash_mb.clamp(HASH_MB_MIN, HASH_MB_MAX));
        e.multi_pv = e.multi_pv.clamp(MULTI_PV_MIN, MULTI_PV_MAX);
        e.depth_limit = e.depth_limit.map(|d| d.clamp(DEPTH_MIN, DEPTH_MAX));
        e.move_time_ms = e
            .move_time_ms
            .map(|t| t.clamp(MOVE_TIME_MIN_MS, MOVE_TIME_MAX_MS));
        e.skill_level = e.skill_level.map(|s| s.min(SKILL_MAX));
        e.min_display_depth = e.min_display_depth.min(MIN_DISPLAY_DEPTH_MAX);
        self.capture.fps = self.capture.fps.clamp(FPS_MIN, FPS_MAX);
        if self.capture.target_id.as_deref().is_some_and(str::is_empty) {
            self.capture.target_id = None;
        }
        // The arrow count is edited from Rust by the cycle hotkey, so keep it sane here.
        if let Some(Value::Object(arrows)) = self.extra.get_mut("arrows")
            && let Some(count) = arrows.get("count").and_then(Value::as_f64)
        {
            let clamped = (count.round().max(ARROWS_MIN as f64) as u64).min(ARROWS_MAX);
            arrows.insert("count".into(), json!(clamped));
        }
    }

    /// Current `arrows.count`, defaulting to 3 when absent.
    pub fn arrow_count(&self) -> u64 {
        self.extra
            .get("arrows")
            .and_then(|a| a.get("count"))
            .and_then(Value::as_u64)
            .unwrap_or(3)
            .clamp(ARROWS_MIN, ARROWS_MAX)
    }

    pub fn set_arrow_count(&mut self, count: u64) {
        let count = count.clamp(ARROWS_MIN, ARROWS_MAX);
        let arrows = self.extra.entry("arrows").or_insert_with(|| json!({}));
        if let Value::Object(map) = arrows {
            map.insert("count".into(), json!(count));
        }
    }

    /// Factory defaults. The fair-play acknowledgement is kept so a reset cannot
    /// silently revoke consent the user already gave.
    pub fn factory_reset(&self) -> Settings {
        Settings {
            accepted_fair_play: self.accepted_fair_play,
            ..Settings::default()
        }
    }

    pub fn hud_visible(&self) -> bool {
        self.extra
            .get("hud")
            .and_then(|h| h.get("show"))
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    pub fn set_hud_visible(&mut self, show: bool) {
        let hud = self.extra.entry("hud").or_insert_with(|| json!({}));
        if let Value::Object(map) = hud {
            map.insert("show".into(), json!(show));
        }
    }

    /// Parse arbitrary JSON into settings: missing fields take defaults, values
    /// of the wrong JSON kind are replaced by defaults, unknown fields are kept.
    pub fn from_value(mut value: Value) -> Result<Settings, String> {
        fill_defaults(&mut value, &default_json());
        let mut s: Settings = serde_json::from_value(value).map_err(|e| e.to_string())?;
        s.validate();
        Ok(s)
    }

    #[cfg(test)]
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("settings serialise")
    }
}

fn json_kind(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

/// Recursively add keys missing from `target` and replace values whose JSON
/// kind differs from the default's (a `null` default accepts any kind).
fn fill_defaults(target: &mut Value, defaults: &Value) {
    match (target, defaults) {
        (Value::Object(t), Value::Object(d)) => {
            for (k, dv) in d {
                match t.get_mut(k) {
                    Some(tv) => {
                        if matches!(dv, Value::Null) {
                            continue;
                        }
                        if json_kind(tv) != json_kind(dv) {
                            *tv = dv.clone();
                        } else {
                            fill_defaults(tv, dv);
                        }
                    }
                    None => {
                        t.insert(k.clone(), dv.clone());
                    }
                }
            }
        }
        (t, d) => {
            if json_kind(t) != json_kind(d) {
                *t = d.clone();
            }
        }
    }
}

/// Write `bytes` to `path` atomically: temp file in the same directory, fsync, rename.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Outcome of reading the settings file.
#[derive(Debug)]
pub struct LoadOutcome {
    pub settings: Settings,
    /// Set when the file was unreadable and defaults were substituted.
    pub recovered_from: Option<String>,
}

/// Load settings from `path`. A missing file yields defaults; an unparsable
/// file is moved to `<path>.bak` and defaults are returned.
pub fn load_from(path: &Path) -> LoadOutcome {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return LoadOutcome {
                settings: Settings::default(),
                recovered_from: None,
            };
        }
        Err(e) => return recover(path, format!("cannot read settings: {e}")),
    };
    let parsed = serde_json::from_str::<Value>(&text)
        .map_err(|e| e.to_string())
        .and_then(Settings::from_value);
    match parsed {
        Ok(settings) => LoadOutcome {
            settings,
            recovered_from: None,
        },
        Err(e) => recover(path, format!("settings file is corrupt: {e}")),
    }
}

fn recover(path: &Path, reason: String) -> LoadOutcome {
    let mut bak = path.as_os_str().to_owned();
    bak.push(".bak");
    let bak = PathBuf::from(bak);
    let _ = fs::remove_file(&bak);
    if fs::rename(path, &bak).is_err() {
        let _ = fs::copy(path, &bak);
    }
    LoadOutcome {
        settings: Settings::default(),
        recovered_from: Some(reason),
    }
}

pub fn save_to(path: &Path, settings: &Settings) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    text.push('\n');
    atomic_write(path, text.as_bytes()).map_err(|e| format!("could not save settings: {e}"))
}

/// Thread-safe owner of the live settings plus their file location.
pub struct SettingsStore {
    path: PathBuf,
    current: RwLock<Settings>,
}

impl SettingsStore {
    /// Open (or create) the store in `dir`. Returns the store and a warning if
    /// the previous file had to be discarded.
    pub fn open(dir: &Path) -> (SettingsStore, Option<String>) {
        let path = dir.join(SETTINGS_FILE);
        let outcome = load_from(&path);
        let store = SettingsStore {
            path,
            current: RwLock::new(outcome.settings),
        };
        (store, outcome.recovered_from)
    }

    pub fn get(&self) -> Settings {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Validate, persist and install `settings`; returns the sanitised value.
    pub fn set(&self, mut settings: Settings) -> Result<Settings, String> {
        settings.validate();
        save_to(&self.path, &settings)?;
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = settings.clone();
        Ok(settings)
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Self-cleaning unique temp directory (no extra dependency).
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> TempDir {
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!(
                "fishy-test-{}-{}-{tag}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::TempDir;
    use super::*;

    #[test]
    fn defaults_serialise_to_the_full_ts_shape() {
        let v = Settings::default().to_value();
        for key in [
            "version",
            "acceptedFairPlay",
            "site",
            "arrows",
            "labels",
            "evalBar",
            "hud",
            "engine",
            "capture",
            "theme",
            "hotkeys",
        ] {
            assert!(v.get(key).is_some(), "missing {key}");
        }
        assert_eq!(v["site"], "chessCom");
        assert_eq!(v["arrows"]["rankColors"].as_array().unwrap().len(), 5);
        assert_eq!(v["labels"]["classes"].as_object().unwrap().len(), 8);
        assert_eq!(v["labels"]["tags"].as_object().unwrap().len(), 12);
        assert_eq!(v["capture"]["targetId"], Value::Null);
        assert_eq!(v["engine"]["depthLimit"], Value::Null);
    }

    #[test]
    fn auto_engine_and_display_depth_round_trip_and_default() {
        let d = Settings::default();
        assert!(d.engine.auto);
        assert_eq!(d.engine.min_display_depth, 12);
        let v = d.to_value();
        assert_eq!(v["engine"]["auto"], true);
        assert_eq!(v["engine"]["minDisplayDepth"], 12);
        // Settings saved before the fields existed behave as auto with the default depth.
        let mut old = default_json();
        old["engine"].as_object_mut().unwrap().remove("auto");
        old["engine"]
            .as_object_mut()
            .unwrap()
            .remove("minDisplayDepth");
        let s = Settings::from_value(old).unwrap();
        assert!(s.engine.auto);
        assert_eq!(s.engine.min_display_depth, 12);
        // Explicit values survive, and the depth is clamped.
        let mut v = default_json();
        v["engine"]["auto"] = json!(false);
        v["engine"]["minDisplayDepth"] = json!(0);
        let s = Settings::from_value(v).unwrap();
        assert!(!s.engine.auto);
        assert_eq!(s.engine.min_display_depth, 0);
        assert_eq!(s.to_value()["engine"]["auto"], false);
        let mut v = default_json();
        v["engine"]["minDisplayDepth"] = json!(500);
        assert_eq!(
            Settings::from_value(v).unwrap().engine.min_display_depth,
            40
        );
    }

    #[test]
    fn unknown_fields_round_trip_losslessly() {
        let mut v = default_json();
        v["futureTopLevel"] = json!({ "a": [1, 2, 3] });
        v["engine"]["futureEngineKnob"] = json!("x");
        v["capture"]["region"] = json!({ "w": 1 });
        v["hotkeys"]["extraKey"] = json!("Alt+Q");
        v["arrows"]["style"] = json!("pill");
        v["labels"]["classes"]["best"]["color"] = json!("#123456");
        let s = Settings::from_value(v.clone()).unwrap();
        assert_eq!(s.extra["futureTopLevel"], json!({ "a": [1, 2, 3] }));
        assert_eq!(s.engine.extra["futureEngineKnob"], "x");
        let out = s.to_value();
        assert_eq!(out["futureTopLevel"], v["futureTopLevel"]);
        assert_eq!(out["engine"]["futureEngineKnob"], "x");
        assert_eq!(out["capture"]["region"], v["capture"]["region"]);
        assert_eq!(out["hotkeys"]["extraKey"], "Alt+Q");
        assert_eq!(out["arrows"]["style"], "pill");
        assert_eq!(out["labels"]["classes"]["best"]["color"], "#123456");
    }

    #[test]
    fn file_round_trip_is_lossless() {
        let dir = TempDir::new("roundtrip");
        let path = dir.path().join(SETTINGS_FILE);
        let mut s = Settings {
            accepted_fair_play: true,
            ..Settings::default()
        };
        s.extra.insert("brandNew".into(), json!({ "k": true }));
        save_to(&path, &s).unwrap();
        let loaded = load_from(&path);
        assert!(loaded.recovered_from.is_none());
        assert_eq!(loaded.settings, s);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\n  \"site\""), "pretty printed");
    }

    #[test]
    fn missing_sections_and_wrong_kinds_fall_back_to_defaults() {
        let v =
            json!({ "engine": { "threads": 1 }, "arrows": "oops", "capture": { "fps": "fast" } });
        let s = Settings::from_value(v).unwrap();
        assert_eq!(s.engine.hash_mb, 128);
        assert_eq!(s.capture.fps, 8);
        assert!(s.extra["arrows"].is_object());
        assert_eq!(s.hotkeys.toggle_overlay, "Alt+Space");
    }

    #[test]
    fn orientation_defaults_to_auto_and_round_trips() {
        let d = Settings::default();
        assert_eq!(d.capture.orientation, OrientationMode::Auto);
        // Auto is not written, so the saved shape stays that of the TS defaults.
        assert!(d.to_value()["capture"].get("orientation").is_none());
        let mut v = d.to_value();
        v["capture"]["orientation"] = json!("black");
        let s = Settings::from_value(v).unwrap();
        assert_eq!(s.capture.orientation, OrientationMode::Black);
        assert_eq!(s.to_value()["capture"]["orientation"], "black");
        // An explicit "auto" from the UI, and anything unknown, mean auto.
        for bad in [json!("auto"), json!("sideways"), json!(7), json!(null)] {
            let mut v = d.to_value();
            v["capture"]["orientation"] = bad;
            let s = Settings::from_value(v).unwrap();
            assert_eq!(s.capture.orientation, OrientationMode::Auto);
        }
        // Settings files from before the field existed still load.
        let s = Settings::from_value(json!({ "capture": { "fps": 12 } })).unwrap();
        assert_eq!(
            (s.capture.fps, s.capture.orientation),
            (12, OrientationMode::Auto)
        );
    }

    #[test]
    fn values_are_clamped() {
        let mut s = Settings::default();
        s.capture.fps = 500;
        s.engine.multi_pv = 99;
        s.engine.threads = 0;
        s.engine.hash_mb = 1000;
        s.engine.depth_limit = Some(0);
        s.engine.move_time_ms = Some(1);
        s.engine.skill_level = Some(99);
        s.set_arrow_count(0);
        s.validate_with_cpus(4);
        assert_eq!(s.capture.fps, FPS_MAX);
        assert_eq!(s.engine.multi_pv, MULTI_PV_MAX);
        assert_eq!(s.engine.threads, 1);
        assert_eq!(s.engine.hash_mb, 512, "rounded down to a power of two");
        assert_eq!(s.engine.depth_limit, Some(DEPTH_MIN));
        assert_eq!(s.engine.move_time_ms, Some(MOVE_TIME_MIN_MS));
        assert_eq!(s.engine.skill_level, Some(SKILL_MAX));
        assert_eq!(s.arrow_count(), 1);

        s.capture.fps = 0;
        s.engine.multi_pv = 0;
        s.engine.threads = 64;
        s.engine.hash_mb = u32::MAX;
        s.validate_with_cpus(4);
        assert_eq!(s.capture.fps, FPS_MIN);
        assert_eq!(s.engine.multi_pv, MULTI_PV_MIN);
        assert_eq!(s.engine.threads, 4);
        assert_eq!(s.engine.hash_mb, HASH_MB_MAX);

        s.extra.get_mut("arrows").unwrap()["count"] = json!(42);
        s.validate_with_cpus(4);
        assert_eq!(s.arrow_count(), 5);
    }

    #[test]
    fn corrupt_file_is_backed_up_and_defaults_used() {
        let dir = TempDir::new("corrupt");
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(&path, "{ this is not json").unwrap();
        let out = load_from(&path);
        assert!(out.recovered_from.is_some());
        assert_eq!(out.settings, Settings::default());
        let bak = dir.path().join("settings.json.bak");
        assert_eq!(std::fs::read_to_string(bak).unwrap(), "{ this is not json");
        assert!(!path.exists());

        // And the store can then save over the top.
        let (store, warning) = SettingsStore::open(dir.path());
        assert!(warning.is_none());
        let mut s = store.get();
        s.capture.fps = 12;
        store.set(s).unwrap();
        let (again, _) = SettingsStore::open(dir.path());
        assert_eq!(again.get().capture.fps, 12);
    }

    #[test]
    fn typed_section_with_bad_value_recovers() {
        let dir = TempDir::new("badvalue");
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(&path, r#"{"engine":{"threads":-3}}"#).unwrap();
        let out = load_from(&path);
        assert!(out.recovered_from.is_some());
        assert!(dir.path().join("settings.json.bak").exists());
    }

    #[test]
    fn reset_keeps_fair_play_acceptance() {
        let mut s = Settings {
            accepted_fair_play: true,
            ..Settings::default()
        };
        s.capture.fps = 20;
        let r = s.factory_reset();
        assert!(r.accepted_fair_play);
        assert_eq!(r.capture.fps, 8);
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_files() {
        let dir = TempDir::new("atomic");
        let p = dir.path().join("a.json");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(leftovers.len(), 1);
    }
}
