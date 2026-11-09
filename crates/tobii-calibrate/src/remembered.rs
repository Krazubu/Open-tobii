//! The monitor the last calibration was made on, kept in the per-user Tobii
//! directory so that the next one opens there by default. The saved display
//! area alone cannot always say which monitor it was: identical monitors
//! report identical EDIDs.
//!
//! The file is text, the monitor's name as the window system gives it:
//!
//! ```text
//! # The monitor tobii-calibrate last calibrated on.
//! DP-3
//! ```

use std::fs;
use std::io;
use std::path::PathBuf;

/// File name in the per-user Tobii directory.
const FILE_NAME: &str = "calibration-monitor";

/// Where it is kept, if there is a per-user directory.
fn path() -> Option<PathBuf> {
    tobii_calib::store::config_dir().map(|dir| dir.join(FILE_NAME))
}

/// The monitor the last calibration was made on, if one is known.
pub(crate) fn monitor() -> Option<String> {
    let path = path()?;
    match fs::read_to_string(&path) {
        Ok(text) => from_text(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => {
            tracing::debug!(path = %path.display(), error = %e, "no monitor to go by");
            None
        }
    }
}

/// Keep `name` as the monitor the last calibration was made on. Failing
/// costs only the next calibration's default, so it is logged, not fatal.
pub(crate) fn remember(name: &str) {
    let Some(path) = path() else {
        return;
    };
    let written = path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(&path, to_text(name)));
    match written {
        Ok(()) => tracing::debug!(path = %path.display(), name, "monitor remembered"),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "could not remember the monitor");
        }
    }
}

fn to_text(name: &str) -> String {
    format!("# The monitor tobii-calibrate last calibrated on.\n{name}\n")
}

/// The name: the first line that is neither empty nor a comment.
fn from_text(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_reads_back() {
        assert_eq!(from_text(&to_text("DP-3")).as_deref(), Some("DP-3"));
        assert_eq!(
            from_text("\n# a comment\n  HDMI-1  \n").as_deref(),
            Some("HDMI-1")
        );
        assert_eq!(from_text("# nothing else\n"), None);
        assert_eq!(from_text(""), None);
    }
}
