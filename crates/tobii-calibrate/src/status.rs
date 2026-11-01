//! What the tracker runs on when tobiid starts it — the user's calibration
//! or the built-in one, their display area or the built-in one — and
//! whether it is plugged in. `--status` prints it, and `--remind` reminds of
//! what is missing for gaze on the screen.
//!
//! Everything is read from the files tobiid reads and from sysfs. Nothing
//! here connects to the daemon, which would start it and open the tracker.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tobii_calib::display;
use tobii_calib::store::{self, Location};

/// Where the kernel lists USB devices.
const USB_DEVICES: &str = "/sys/bus/usb/devices";

/// The Eye Tracker 5 on the bus, as sysfs spells it.
const VENDOR: &str = "2104";
const PRODUCT: &str = "0313";

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

/// What tobiid starts the tracker with, as far as this process can tell.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Status {
    pub(crate) calibration: Calibration,
    pub(crate) area: Area,
    /// `TOBII_CALIBRATION` here; tobiid reads its own environment.
    pub(crate) calibration_env: Option<OsString>,
    /// `TOBII_DISPLAY_MM` here; tobiid reads its own environment.
    pub(crate) display_env: Option<String>,
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
    /// Read it all now.
    pub(crate) fn read() -> Self {
        let non_empty = |v: &OsString| !v.is_empty();
        Self {
            calibration: calibration(),
            area: area(),
            calibration_env: std::env::var_os(store::ENV_OVERRIDE).filter(non_empty),
            display_env: std::env::var("TOBII_DISPLAY_MM")
                .ok()
                .filter(|v| !v.is_empty()),
            tracker: tracker_on_bus(Path::new(USB_DEVICES)),
        }
    }

    /// What is missing for gaze on the screen, if anything. A display area
    /// `TOBII_DISPLAY_MM` gives is taken to reach the daemon too.
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
        if let Some(value) = &self.calibration_env {
            let value = value.to_string_lossy();
            line(
                &mut out,
                "environment",
                &format!(
                    "{}={value} here; tobiid goes by its own",
                    store::ENV_OVERRIDE
                ),
            );
        }
        if let Some(value) = &self.display_env {
            line(
                &mut out,
                "environment",
                &format!(
                    "TOBII_DISPLAY_MM={value} here; tobiid uses its own while no area is saved"
                ),
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
                    .map_or_else(String::new, |p| format!(" (none at {})", p.display()));
                format!("the built-in one, the author's{none_at}")
            }
            Calibration::Unusable { path, why } => format!(
                "the built-in one, the author's: {} is skipped ({why})",
                path.display()
            ),
            Calibration::Own {
                path,
                id,
                points,
                saved,
            } => format!(
                "yours: id {id}, {points} points{} ({})",
                saved_on(*saved),
                path.display()
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
                    .map_or_else(String::new, |p| format!(" (none at {})", p.display()));
                format!("{built_in}, not your monitor's{none_at}")
            }
            Area::Unusable { path, why } => {
                format!("{built_in}: {} is skipped ({why})", path.display())
            }
            Area::Saved {
                path,
                size_mm: (w, h),
                saved,
            } => format!(
                "yours: {w:.1} x {h:.1} mm ({}\"){} ({})",
                inches(*w, *h),
                saved_on(*saved),
                path.display()
            ),
        }
    }
}

/// The diagonal of a `w` x `h` mm screen, in inches to one decimal.
fn inches(w: f64, h: f64) -> String {
    format!("{:.1}", w.hypot(h) / 25.4)
}

/// ", saved <date>" for a file's time, if it has one.
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

fn calibration() -> Calibration {
    let chosen = std::env::var_os(store::ENV_OVERRIDE).is_some_and(|v| v == store::EMBEDDED);
    match store::configured() {
        Location::Embedded if chosen => Calibration::BuiltInChosen,
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

fn area() -> Area {
    let Some(path) = display::default_path() else {
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
