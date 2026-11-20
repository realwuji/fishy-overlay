//! Per-site piece templates, stored as JSON in the app data directory.
//!
//! Templates are tiny numeric descriptors learned from the user's own screen;
//! they never contain captured frames or image data.

use crate::settings::atomic_write;
use fishy_vision::{Site, Templates};
use std::path::{Path, PathBuf};

pub fn path_for(dir: &Path, site: Site) -> PathBuf {
    let slug = match site {
        Site::ChessCom => "chessCom",
        Site::Lichess => "lichess",
        Site::Generic => "generic",
    };
    dir.join(format!("templates-{slug}.json"))
}

/// Load templates for `site`; missing or unreadable files mean "not calibrated".
pub fn load(dir: &Path, site: Site) -> Option<Templates> {
    let text = std::fs::read_to_string(path_for(dir, site)).ok()?;
    let t: Templates = serde_json::from_str(&text).ok()?;
    if t.data.is_null() { None } else { Some(t) }
}

/// Where the active templates came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Learned by the user (`calibrate_start_position`); always takes precedence.
    User,
    /// Bundled with the app for the site's default look (chess.com first).
    BuiltIn,
}

impl Source {
    /// Human-readable name used in status messages.
    pub fn label(self, site: Site) -> String {
        match self {
            Source::User => "your calibration".to_string(),
            Source::BuiltIn => format!("built-in {} pieces", site_name(site)),
        }
    }
}

pub fn site_name(site: Site) -> &'static str {
    match site {
        Site::ChessCom => "chess.com",
        Site::Lichess => "lichess",
        Site::Generic => "generic",
    }
}

/// Saved calibration if any, otherwise the bundled templates for `site`.
pub fn resolve(dir: &Path, site: Site) -> Option<(Templates, Source)> {
    if let Some(t) = load(dir, site) {
        return Some((t, Source::User));
    }
    Templates::builtin(site).map(|t| (t, Source::BuiltIn))
}

pub fn save(dir: &Path, site: Site, templates: &Templates) -> Result<(), String> {
    let bytes = serde_json::to_vec(templates).map_err(|e| e.to_string())?;
    atomic_write(&path_for(dir, site), &bytes)
        .map_err(|e| format!("could not save piece templates: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::testutil::TempDir;

    #[test]
    fn round_trip_per_site() {
        let dir = TempDir::new("templates");
        assert!(load(dir.path(), Site::ChessCom).is_none());
        let t = Templates {
            data: serde_json::json!({ "k": [1, 2, 3] }),
        };
        save(dir.path(), Site::ChessCom, &t).unwrap();
        assert_eq!(load(dir.path(), Site::ChessCom).unwrap().data, t.data);
        assert!(
            load(dir.path(), Site::Lichess).is_none(),
            "templates are per site"
        );
    }

    #[test]
    fn saved_calibration_beats_builtin() {
        let dir = TempDir::new("templates-resolve");
        let (_, src) = resolve(dir.path(), Site::ChessCom).expect("builtin chess.com");
        assert_eq!(src, Source::BuiltIn);
        assert!(resolve(dir.path(), Site::Lichess).is_none());
        let t = Templates {
            data: serde_json::json!({ "k": [1] }),
        };
        save(dir.path(), Site::ChessCom, &t).unwrap();
        let (got, src) = resolve(dir.path(), Site::ChessCom).unwrap();
        assert_eq!(src, Source::User);
        assert_eq!(got.data, t.data);
        assert_eq!(
            Source::BuiltIn.label(Site::ChessCom),
            "built-in chess.com pieces"
        );
        assert_eq!(Source::User.label(Site::ChessCom), "your calibration");
    }

    #[test]
    fn garbage_or_null_means_uncalibrated() {
        let dir = TempDir::new("templates-bad");
        std::fs::write(path_for(dir.path(), Site::Generic), "not json").unwrap();
        assert!(load(dir.path(), Site::Generic).is_none());
        std::fs::write(path_for(dir.path(), Site::Generic), r#"{"data":null}"#).unwrap();
        assert!(load(dir.path(), Site::Generic).is_none());
    }
}
