//! Small shared helpers.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Run `f`, converting a panic into an error string so a bug in one component
/// can never take down the app or leave a worker thread dead without a report.
///
/// Note: this only helps when panics unwind (`panic = "unwind"`).
pub fn guard<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(panic_message)
}

/// Like [`guard`] for fallible closures.
pub fn guard_result<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    guard(f).and_then(|r| r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_passes_values_through() {
        assert_eq!(guard(|| 5), Ok(5));
        assert_eq!(
            guard_result(|| Err::<(), _>("nope".into())),
            Err("nope".into())
        );
    }

    #[test]
    fn guard_converts_panics() {
        let r = guard(|| -> u8 { panic!("boom {}", 1) });
        assert_eq!(r, Err("boom 1".to_string()));
        let r = guard_result::<u8>(|| panic!("static"));
        assert_eq!(r, Err("static".to_string()));
    }
}
