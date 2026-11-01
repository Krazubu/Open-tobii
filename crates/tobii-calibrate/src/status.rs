//! What the tracker runs on when tobiid starts it — the user's calibration
//! or the built-in one, their display area or the built-in one — and
//! whether it is plugged in. `--status` prints it, and `--remind` reminds of
//! what is missing for gaze on the screen.
//!
//! Everything is read from the files tobiid reads, in the environment
//! systemd starts it with, and from sysfs. Nothing here connects to the
//! daemon, which would start it and open the tracker.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tobii_calib::display;
use tobii_calib::store::{self, Location};

/// Where the kernel lists USB devices.
const USB_DEVICES: &str = "/sys/bus/usb/devices";

/// The Eye Tracker 5 on the bus, as sysfs spells it.
const VENDOR: &str = "2104";
const PRODUCT: &str = "0313";

/// The variable that gives tobiid a display area while none is saved.
const DISPLAY_MM: &str = "TOBII_DISPLAY_MM";

/// How much later than the calibration the display area may be saved and
/// still count as the one it was made on: a session that is kept saves
/// both at its end.
const SAVED_TOGETHER: Duration = Duration::from_secs(60);

/// The calibration tobiid uploads at the tracker's next start.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Calibration {
    /// `TOBII_CALIBRATION=embedded`: the built-in one, by choice.
    BuiltInChosen,
    /// None of the user's: the built-in one, the author's. `path` is where
    /// theirs would be saved (none without a home directory).
    BuiltIn { path: Option<PathBuf> },
    /// A file tobiid skips for the built-in one: unreadable, or not a
    /// calibration.
    Unusable { path: PathBuf, why: String },
    /// The user's own.
    Own {
        path: PathBuf,
        id: u32,
        points: usize,
        saved: Option<SystemTime>,
    },
}

/// The display area tobiid configures at the tracker's next start.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Area {
    /// None saved: the built-in one ([`display::BUILT_IN_MM`]), unless
    /// `TOBII_DISPLAY_MM` reaches the daemon. `path` is where it would be.
    BuiltIn { path: Option<PathBuf> },
    /// A file tobiid skips for the built-in area.
    Unusable { path: PathBuf, why: String },
    /// The user's, as the display setup (or a Stream Engine application)
    /// set it.
    Saved {
        path: PathBuf,
        size_mm: (f64, f64),
        saved: Option<SystemTime>,
    },
}

/// Where the variables tobiid reads were taken from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnvSource {
    /// systemd's account of what it starts tobiid with (see
    /// [`daemon_environment`]).
    Systemd,
    /// This process's own: systemd did not answer.
    ThisProcess,
}

/// What tobiid starts the tracker with, as far as this process can tell.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Status {
    pub(crate) calibration: Calibration,
    pub(crate) area: Area,
    /// `TOBII_CALIBRATION` as tobiid gets it.
    pub(crate) calibration_env: Option<OsString>,
    /// `TOBII_DISPLAY_MM` as tobiid gets it.
    pub(crate) display_env: Option<String>,
    /// Where those were taken from.
    pub(crate) env_source: EnvSource,
    /// Those two as this process has them where tobiid does not, or not
    /// alike: `NAME=value`.
    pub(crate) here_only: Vec<String>,
    /// The tracker's place on the USB bus (`1-10.1`), if it is plugged in.
    pub(crate) tracker: Option<String>,
}

/// What a setup lacks for gaze on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Missing {
    /// No calibration of the user's: the tracker runs on the author's.
    Calibration,
    /// The user's calibration, but no display area saved: it was made on
    /// the area the tracker had then (with `--no-display-setup`, or before
    /// 0.4.1), and gaze is scaled to the built-in one.
    DisplayArea,
}

impl Status {
    /// Read it all now, in the environment tobiid gets from systemd (this
    /// process's, if systemd does not say).
    pub(crate) fn read() -> Self {
        let daemon = daemon_environment();
        let env_source = if daemon.is_some() {
            EnvSource::Systemd
        } else {
            EnvSource::ThisProcess
        };
        let var = |name: &str| {
            match &daemon {
                Some(env) => env.get(name).cloned(),
                None => std::env::var_os(name),
            }
            .filter(|v| !v.is_empty())
        };
        let here_only = [store::ENV_OVERRIDE, DISPLAY_MM]
            .into_iter()
            .filter_map(|name| {
                let here = std::env::var_os(name).filter(|v| !v.is_empty())?;
                (var(name).as_ref() != Some(&here))
                    .then(|| format!("{name}={}", here.to_string_lossy()))
            })
            .collect();
        Self {
            calibration: calibration(&var),
            area: area(&var),
            calibration_env: var(store::ENV_OVERRIDE),
            display_env: var(DISPLAY_MM).map(|v| v.to_string_lossy().into_owned()),
            env_source,
            here_only,
            tracker: tracker_on_bus(Path::new(USB_DEVICES)),
        }
    }

    /// What is missing for gaze on the screen, if anything. A display area
    /// `TOBII_DISPLAY_MM` gives tobiid counts as one.
    pub(crate) fn missing(&self) -> Option<Missing> {
        match (&self.calibration, &self.area) {
            (Calibration::BuiltIn { .. } | Calibration::Unusable { .. }, _) => {
                Some(Missing::Calibration)
            }
            (Calibration::Own { .. }, Area::BuiltIn { .. } | Area::Unusable { .. })
                if self.display_env.is_none() =>
            {
                Some(Missing::DisplayArea)
            }
            _ => None,
        }
    }

    /// Whether the display area was saved after the calibration was made,
    /// so that the calibration was made on an area the tracker has no more.
    /// tobiid writes the area file only when the area changes.
    pub(crate) fn area_changed_since(&self) -> bool {
        let (
            Calibration::Own {
                saved: Some(calibrated),
                ..
            },
            Area::Saved {
                saved: Some(set), ..
            },
        ) = (&self.calibration, &self.area)
        else {
            return false;
        };
        set.duration_since(*calibrated)
            .is_ok_and(|later| later > SAVED_TOGETHER)
    }

    /// The report `--status` prints.
    pub(crate) fn report(&self) -> String {
        let mut out = String::new();
        let line = |out: &mut String, label: &str, text: &str| {
            let _ = writeln!(out, "{label:<14}{text}");
        };
        line(&mut out, "calibration", &self.describe_calibration());
        line(&mut out, "display area", &self.describe_area());
        let tracker = self.tracker.as_ref().map_or_else(
            || "not plugged in".to_owned(),
            |at| format!("plugged in (USB {at})"),
        );
        line(&mut out, "tracker", &tracker);
        let from = match self.env_source {
            EnvSource::Systemd => "tobiid gets it",
            EnvSource::ThisProcess => "here; systemd does not say what tobiid gets",
        };
        if let Some(value) = &self.calibration_env {
            let value = value.to_string_lossy();
            line(
                &mut out,
                "environment",
                &format!("{}={value} ({from})", store::ENV_OVERRIDE),
            );
        }
        if let Some(value) = &self.display_env {
            line(
                &mut out,
                "environment",
                &format!("{DISPLAY_MM}={value} ({from}; it counts while no area is saved)"),
            );
        }
        for here in &self.here_only {
            line(
                &mut out,
                "environment",
                &format!("{here} here, which tobiid does not get"),
            );
        }
        let advice = match self.missing() {
            Some(Missing::Calibration) => {
                Some("gaze on the screen needs a calibration of yours: run tobii-calibrate")
            }
            Some(Missing::DisplayArea) => Some(
                "the calibration was made without the display setup: run tobii-calibrate again",
            ),
            None if self.area_changed_since() => Some(
                "the display area changed after the calibration was made: run tobii-calibrate \
                 again",
            ),
            None => None,
        };
        if let Some(advice) = advice {
            line(&mut out, "to do", advice);
        }
        out
    }

    fn describe_calibration(&self) -> String {
        match &self.calibration {
            Calibration::BuiltInChosen => format!(
                "the built-in one, the author's ({}={})",
                store::ENV_OVERRIDE,
                store::EMBEDDED
            ),
            Calibration::BuiltIn { path } => {
                let none_at = path
                    .as_ref()
                    .map_or_else(String::new, |p| format!(" (none at {})", shown(p)));
                format!("the built-in one, the author's{none_at}")
            }
            Calibration::Unusable { path, why } => format!(
                "the built-in one, the author's: {} is skipped ({why})",
                shown(path)
            ),
            Calibration::Own {
                path,
                id,
                points,
                saved,
            } => format!(
                "yours: id {id}, {points} points{} ({})",
                saved_on(*saved),
                shown(path)
            ),
        }
    }

    fn describe_area(&self) -> String {
        let (w, h) = display::BUILT_IN_MM;
        let built_in = format!("the built-in one, {w:.1} x {h:.1} mm ({}\")", inches(w, h));
        match &self.area {
            Area::BuiltIn { path } => {
                let none_at = path
                    .as_ref()
                    .map_or_else(String::new, |p| format!(" (none at {})", shown(p)));
                format!("{built_in}, not your monitor's{none_at}")
            }
            Area::Unusable { path, why } => {
                format!("{built_in}: {} is skipped ({why})", shown(path))
            }
            Area::Saved {
                path,
                size_mm: (w, h),
                saved,
            } => format!(
                "yours: {w:.1} x {h:.1} mm ({}\"){} ({})",
                inches(*w, *h),
                saved_on(*saved),
                shown(path)
            ),
        }
    }
}

/// `path` as the report shows it: under the home directory as `~/...`,
/// so that a report pasted into an issue does not carry the user's name.
fn shown(path: &Path) -> String {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty());
    home_relative(path, home.as_deref().map(Path::new))
}

fn home_relative(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// The diagonal of a `w` x `h` mm screen, in inches to one decimal.
fn inches(w: f64, h: f64) -> String {
    format!("{:.1}", w.hypot(h) / 25.4)
}

/// `, saved YYYY-MM-DD` for a file's time, if it has one.
fn saved_on(saved: Option<SystemTime>) -> String {
    saved
        .and_then(date)
        .map_or_else(String::new, |d| format!(", saved {d}"))
}

/// The UTC date of `t` as YYYY-MM-DD; none before 1970.
fn date(t: SystemTime) -> Option<String> {
    let days = t.duration_since(UNIX_EPOCH).ok()?.as_secs() / 86_400;
    let (y, m, d) = civil_from_days(i64::try_from(days).ok()?);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// The proleptic Gregorian date `days` after 1970-01-01 (Howard Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// When the file at `path` was last written.
fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The environment tobiid gets from systemd, which starts it: the user
/// manager's, with tobiid.service's own `Environment=` over it (where
/// INSTALL.md §7 sets the daemon's variables, `systemctl --user edit
/// tobiid`). Asking starts nothing. `None` when systemctl does not answer;
/// an `EnvironmentFile=` is not read.
fn daemon_environment() -> Option<HashMap<String, OsString>> {
    let ask = |args: &[&str]| {
        Command::new("systemctl")
            .arg("--user")
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let mut env: HashMap<String, OsString> = ask(&["show-environment"])?
        .lines()
        .filter_map(manager_assignment)
        .collect();
    if let Some(unit) = ask(&[
        "show",
        "--property=Environment",
        "--value",
        "tobiid.service",
    ]) {
        env.extend(unit_words(&unit).into_iter().filter_map(|word| {
            let (name, value) = word.split_once('=')?;
            Some((name.to_owned(), OsString::from(value)))
        }));
    }
    Some(env)
}

/// A `NAME=value` line of `systemctl show-environment`, which writes a
/// value with whitespace or shell characters in it as `$'...'`, C-escaped.
fn manager_assignment(line: &str) -> Option<(String, OsString)> {
    let (name, value) = line.split_once('=')?;
    let value = match value.strip_prefix("$'").and_then(|v| v.strip_suffix('\'')) {
        Some(quoted) => unescape(quoted),
        None => value.to_owned(),
    };
    Some((name.to_owned(), value.into()))
}

/// The C escapes systemd writes; any other backslash stays as written.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(e @ ('\\' | '\'' | '"')) => out.push(e),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The words of `systemctl show --property=Environment --value`: the
/// assignments, space-separated, each with a space or a quote in it in
/// double quotes, backslash-escaped inside.
fn unit_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    // Inside quotes; a word whose quotes held nothing is still a word.
    let (mut quoted, mut started) = (false, false);
    let mut chars = text.trim().chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => word.extend(chars.next()),
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

fn calibration(var: &impl Fn(&str) -> Option<OsString>) -> Calibration {
    match store::configured_by(var) {
        Location::Embedded if store::embedded_chosen_by(var) => Calibration::BuiltInChosen,
        Location::Embedded => Calibration::BuiltIn { path: None },
        Location::File(path) => match store::load(&path) {
            Ok(Some((_, info))) => Calibration::Own {
                saved: modified(&path),
                path,
                id: info.id,
                points: info.points,
            },
            Ok(None) => Calibration::BuiltIn { path: Some(path) },
            Err(e) => Calibration::Unusable {
                path,
                why: e.to_string(),
            },
        },
    }
}

fn area(var: &impl Fn(&str) -> Option<OsString>) -> Area {
    let Some(path) = store::config_dir_by(var).map(|dir| dir.join(display::FILE_NAME)) else {
        return Area::BuiltIn { path: None };
    };
    match display::load(&path) {
        Ok(Some(saved)) => Area::Saved {
            size_mm: display::size_mm(&saved),
            saved: modified(&path),
            path,
        },
        Ok(None) => Area::BuiltIn { path: Some(path) },
        Err(e) => Area::Unusable {
            path,
            why: e.to_string(),
        },
    }
}

/// The tracker's device directory under `devices` (sysfs' USB devices,
/// interfaces aside), by its vendor and product ids.
pub(crate) fn tracker_on_bus(devices: &Path) -> Option<String> {
    let read = |dir: &Path, file: &str| {
        fs::read_to_string(dir.join(file))
            .ok()
            .map(|s| s.trim().to_owned())
    };
    fs::read_dir(devices)
        .ok()?
        .flatten()
        .filter(|entry| !entry.file_name().to_string_lossy().contains(':'))
        .find(|entry| {
            let dir = entry.path();
            read(&dir, "idVendor").as_deref() == Some(VENDOR)
                && read(&dir, "idProduct").as_deref() == Some(PRODUCT)
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own(saved: u64) -> Calibration {
        Calibration::Own {
            path: "/h/.config/tobii/calibration.bin".into(),
            id: 7,
            points: 14,
            saved: Some(UNIX_EPOCH + Duration::from_secs(saved)),
        }
    }

    fn saved_area(saved: u64) -> Area {
        Area::Saved {
            path: "/h/.config/tobii/display-area".into(),
            size_mm: (597.0, 336.0),
            saved: Some(UNIX_EPOCH + Duration::from_secs(saved)),
        }
    }

    fn status(calibration: Calibration, area: Area) -> Status {
        Status {
            calibration,
            area,
            calibration_env: None,
            display_env: None,
            env_source: EnvSource::Systemd,
            here_only: Vec::new(),
            tracker: Some("1-10.1".into()),
        }
    }

    fn built_in_area() -> Area {
        Area::BuiltIn {
            path: Some("/h/.config/tobii/display-area".into()),
        }
    }

    #[test]
    fn a_new_user_lacks_the_calibration_first() {
        let new = status(Calibration::BuiltIn { path: None }, built_in_area());
        let unusable = status(
            Calibration::Unusable {
                path: "/x".into(),
                why: "not a calibration".into(),
            },
            saved_area(0),
        );

        assert_eq!(new.missing(), Some(Missing::Calibration));
        assert_eq!(unusable.missing(), Some(Missing::Calibration));
        assert_eq!(status(own(0), saved_area(0)).missing(), None);
        assert_eq!(
            status(Calibration::BuiltInChosen, built_in_area()).missing(),
            None
        );
    }

    #[test]
    fn a_calibration_without_a_display_area_lacks_the_area() {
        let mut pre_041 = status(own(0), built_in_area());

        assert_eq!(pre_041.missing(), Some(Missing::DisplayArea));
        pre_041.display_env = Some("597x336".into());
        assert_eq!(pre_041.missing(), None, "the variable gives one");
    }

    #[test]
    fn an_area_set_after_the_calibration_is_told_apart() {
        let day = 86_400;

        assert!(status(own(0), saved_area(30 * day)).area_changed_since());
        // Kept together at the end of one session.
        assert!(!status(own(100), saved_area(130)).area_changed_since());
        assert!(!status(own(30 * day), saved_area(0)).area_changed_since());
        assert!(!status(own(0), built_in_area()).area_changed_since());
    }

    #[test]
    fn the_report_says_what_to_do() {
        let new = status(Calibration::BuiltIn { path: None }, built_in_area()).report();
        let done = status(own(1_758_628_800), saved_area(1_758_628_830)).report();
        let stale = status(own(1_758_628_800), saved_area(1_761_559_200)).report();

        assert!(
            new.contains("the built-in one, 633.6 x 334.3 mm (28.2\")"),
            "{new}"
        );
        assert!(new.contains("run tobii-calibrate"), "{new}");
        assert!(
            done.contains("yours: id 7, 14 points, saved 2025-09-23"),
            "{done}"
        );
        assert!(done.contains("597.0 x 336.0 mm (27.0\")"), "{done}");
        assert!(done.contains("plugged in (USB 1-10.1)"), "{done}");
        assert!(!done.contains("to do"), "{done}");
        assert!(stale.contains("run tobii-calibrate again"), "{stale}");
    }

    #[test]
    fn paths_under_home_are_shown_from_it() {
        let home = Path::new("/home/someone");
        let at = |p: &str| home_relative(Path::new(p), Some(home));

        assert_eq!(
            at("/home/someone/.config/tobii/display-area"),
            "~/.config/tobii/display-area"
        );
        assert_eq!(at("/home/someone-else/x"), "/home/someone-else/x");
        assert_eq!(at("/etc/tobii"), "/etc/tobii");
        assert_eq!(home_relative(Path::new("/x"), None), "/x");
    }

    #[test]
    fn the_daemons_variables_are_read_as_systemd_writes_them() {
        assert_eq!(
            manager_assignment("TOBII_DISPLAY_MM=597x336"),
            Some(("TOBII_DISPLAY_MM".into(), "597x336".into()))
        );
        assert_eq!(
            manager_assignment(r"TOBII_CALIBRATION=$'/home/a b/it\'s.bin'"),
            Some(("TOBII_CALIBRATION".into(), "/home/a b/it's.bin".into()))
        );
        assert_eq!(manager_assignment("no assignment"), None);
        // As `systemctl show -p Environment --value` prints a unit's.
        assert_eq!(
            unit_words(r#"A=1 "B=x y" "C=q\"z" D="#),
            ["A=1", "B=x y", r#"C=q"z"#, "D="]
        );
        assert_eq!(unit_words("\n"), Vec::<String>::new());
    }

    #[test]
    fn a_variable_tobiid_does_not_get_is_told_apart() {
        let mut pre_041 = status(own(0), built_in_area());
        pre_041.here_only = vec!["TOBII_DISPLAY_MM=597x336".into()];

        let report = pre_041.report();

        assert_eq!(pre_041.missing(), Some(Missing::DisplayArea));
        assert!(
            report.contains("TOBII_DISPLAY_MM=597x336 here, which tobiid does not get"),
            "{report}"
        );
        pre_041.display_env = Some("597x336".into());
        assert!(
            pre_041.report().contains("(tobiid gets it;"),
            "{}",
            pre_041.report()
        );
    }

    #[test]
    fn dates_are_utc_days() {
        let at = |secs| date(UNIX_EPOCH + Duration::from_secs(secs));

        assert_eq!(at(0).as_deref(), Some("1970-01-01"));
        assert_eq!(at(951_782_400).as_deref(), Some("2000-02-29"));
        assert_eq!(at(1_761_955_199).as_deref(), Some("2025-10-31"));
        assert_eq!(at(1_761_955_200).as_deref(), Some("2025-11-01"));
    }

    /// A scratch directory under the system temp dir, removed on drop.
    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_tracker_is_found_by_its_ids() {
        let scratch = Scratch(
            std::env::temp_dir().join(format!("tobii-calibrate-usb-{}", std::process::id())),
        );
        let device = |name: &str, vendor: &str, product: &str| {
            let dir = scratch.0.join(name);
            fs::create_dir_all(&dir).expect("dir");
            fs::write(dir.join("idVendor"), format!("{vendor}\n")).expect("vendor");
            fs::write(dir.join("idProduct"), format!("{product}\n")).expect("product");
        };
        device("1-2", "046d", "c52b");
        device("1-10.1:1.0", VENDOR, PRODUCT);

        assert_eq!(tracker_on_bus(&scratch.0), None, "an interface is not it");
        device("1-10.1", VENDOR, PRODUCT);
        assert_eq!(tracker_on_bus(&scratch.0).as_deref(), Some("1-10.1"));
        assert_eq!(tracker_on_bus(&scratch.0.join("absent")), None);
    }
}
