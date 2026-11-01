//! `--remind`, which the desktop's autostart runs at every login: with the
//! tracker plugged in and the user's calibration or display area missing
//! (see [`Missing`]), a desktop notification offers to calibrate. At most
//! three times for each of the two, at the first login, a day later and a
//! week after that, and never again once the user says so.
//!
//! The notification goes through `notify-send`, with buttons where it has
//! them (libnotify 0.7.10 and later), as text where it has none, and through
//! `busctl` without it. A calibration it starts runs from here, so that the
//! autostart unit, whose processes systemd may stop with it, outlives it.
//!
//! A greeter's session and one that is not graphical are left alone, and so
//! is a user who points `TOBII_CALIBRATION` somewhere themselves.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tobii_calib::display;
use tobii_calib::store;

use crate::status::{Area, Missing, Status};

/// File in the per-user Tobii directory that keeps what was shown.
const STATE_FILE: &str = "reminders";

/// How long the desktop gets to come up before anything is looked at.
const SETTLE: Duration = Duration::from_secs(20);

/// How many times each reason is reminded of.
const TIMES: u32 = 3;

const DAY: u64 = 86_400;

/// What the notifications go by: the calibration's desktop entry.
const DESKTOP_ENTRY: &str = "tobii-calibrate";
const APP_NAME: &str = "Tobii Calibration";

/// How long after the `shown`th reminder the next one may come.
fn wait_after(shown: u32) -> u64 {
    match shown {
        0 => 0,
        1 => DAY,
        _ => 7 * DAY,
    }
}

/// How many times a reason was reminded of, and when last (Unix seconds).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Shown {
    times: u32,
    last: u64,
}

/// What `--remind` has shown, as its file keeps it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Reminders {
    /// "Don't remind me": no more reminders at all.
    off: bool,
    calibration: Shown,
    display_area: Shown,
}

impl Reminders {
    fn shown(&self, reason: Missing) -> Shown {
        match reason {
            Missing::Calibration => self.calibration,
            Missing::DisplayArea => self.display_area,
        }
    }

    fn shown_mut(&mut self, reason: Missing) -> &mut Shown {
        match reason {
            Missing::Calibration => &mut self.calibration,
            Missing::DisplayArea => &mut self.display_area,
        }
    }

    /// Whether `reason` is to be reminded of at `now`.
    fn due(&self, reason: Missing, now: u64) -> bool {
        let shown = self.shown(reason);
        !self.off
            && shown.times < TIMES
            && now >= shown.last.saturating_add(wait_after(shown.times))
    }

    fn record(&mut self, reason: Missing, now: u64) {
        let shown = self.shown_mut(reason);
        shown.times = shown.times.saturating_add(1);
        shown.last = now;
    }

    fn to_text(&self) -> String {
        let mut s = String::from(
            "# What tobii-calibrate --remind has shown. Delete this file to be reminded\n\
             # again; \"off\" is the \"Don't remind me\" button.\n",
        );
        if self.off {
            s.push_str("off\n");
        }
        for (name, shown) in [
            ("calibration", self.calibration),
            ("display-area", self.display_area),
        ] {
            if shown.times > 0 {
                let _ = writeln!(s, "{name} {} {}", shown.times, shown.last);
            }
        }
        s
    }

    /// Read leniently: a line it does not know is skipped.
    fn from_text(text: &str) -> Self {
        let mut r = Self::default();
        for line in text.lines().map(str::trim) {
            let mut words = line.split_whitespace();
            let shown = match words.next() {
                Some("off") => {
                    r.off = true;
                    continue;
                }
                Some("calibration") => &mut r.calibration,
                Some("display-area") => &mut r.display_area,
                _ => continue,
            };
            let mut number = || words.next().and_then(|w| w.parse::<u64>().ok());
            if let (Some(times), Some(last)) = (number(), number()) {
                *shown = Shown {
                    times: u32::try_from(times).unwrap_or(u32::MAX),
                    last,
                };
            }
        }
        r
    }
}

/// Where what was shown is kept, if there is a per-user directory.
fn state_path() -> Option<PathBuf> {
    store::config_dir().map(|dir| dir.join(STATE_FILE))
}

fn load(path: &Path) -> Reminders {
    match fs::read_to_string(path) {
        Ok(text) => Reminders::from_text(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Reminders::default(),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "reading what was reminded of");
            Reminders::default()
        }
    }
}

fn save(path: &Path, reminders: &Reminders) -> Result<()> {
    store::write_atomic(path, reminders.to_text().as_bytes())
        .with_context(|| format!("could not write {}", path.display()))
}

/// The line `--status` adds about the reminders.
pub(crate) fn report_line() -> String {
    let Some(path) = state_path() else {
        return String::new();
    };
    let r = load(&path);
    let text = if r.off {
        "off (\"Don't remind me\")".to_owned()
    } else {
        let shown: Vec<String> = [
            ("calibration", r.calibration),
            ("display area", r.display_area),
        ]
        .into_iter()
        .filter(|(_, s)| s.times > 0)
        .map(|(name, s)| format!("{name} {} of {TIMES}", s.times.min(TIMES)))
        .collect();
        if shown.is_empty() {
            "on, none shown yet".to_owned()
        } else {
            format!("on, shown so far: {}", shown.join(", "))
        }
    };
    format!("{:<14}{text}\n", "reminders")
}

/// The languages the reminder speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    English,
    Russian,
}

/// The language of the messages, as gettext would pick it from `var`
/// (the environment): `LANGUAGE`'s list first, unless the locale is C,
/// then the locale of `LC_ALL`, `LC_MESSAGES` or `LANG`; English for any
/// other.
fn language(var: impl Fn(&str) -> Option<String>) -> Language {
    let set = |name: &str| var(name).filter(|v| !v.is_empty());
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(set)
        .unwrap_or_default();
    if locale.is_empty() || locale == "C" || locale == "POSIX" || locale.starts_with("C.") {
        return Language::English;
    }
    let listed = set("LANGUAGE").unwrap_or_default();
    listed
        .split(':')
        .chain([locale.as_str()])
        .find_map(|tag| {
            if tag.starts_with("ru") {
                Some(Language::Russian)
            } else if tag.starts_with("en") {
                Some(Language::English)
            } else {
                None
            }
        })
        .unwrap_or(Language::English)
}

/// What a reminder says.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Texts {
    summary: &'static str,
    body: String,
    calibrate: &'static str,
    never: &'static str,
    /// What to do instead of the button, where there are none.
    by_hand: &'static str,
}

/// The reminder for `reason`; `area_built_in` when the tracker also runs on
/// the built-in display area (the user's may be saved without a
/// calibration: after `--reset`, say).
fn texts(reason: Missing, area_built_in: bool, language: Language) -> Texts {
    let (w, h) = display::BUILT_IN_MM;
    // A decimal comma in Russian.
    let (w_ru, h_ru) = (
        format!("{w:.1}").replace('.', ","),
        format!("{h:.1}").replace('.', ","),
    );
    match (language, reason) {
        (Language::English, reason) => Texts {
            summary: match reason {
                Missing::Calibration => "Calibrate your eye tracker",
                Missing::DisplayArea => "Calibrate your eye tracker again",
            },
            body: match reason {
                Missing::Calibration => format!(
                    "Your Tobii Eye Tracker 5 runs on a calibration made for someone else{}, so \
                     gaze on the screen lands away from where you look. Calibrating takes about \
                     a minute; head tracking does not need it.",
                    if area_built_in {
                        ", and on a display area that is not your monitor's"
                    } else {
                        ""
                    }
                ),
                Missing::DisplayArea => format!(
                    "Your calibration was made without the display setup (with \
                     --no-display-setup, or before tobii-linux 0.4.1), so gaze is scaled to a \
                     {w:.1} × {h:.1} mm screen rather than yours. Calibrating again takes about \
                     a minute."
                ),
            },
            calibrate: "Calibrate",
            never: "Don't remind me",
            by_hand: "Open \u{201c}Tobii Calibration\u{201d} among your applications, or run \
                      tobii-calibrate.",
        },
        (Language::Russian, reason) => Texts {
            summary: match reason {
                Missing::Calibration => "Откалибруйте айтрекер",
                Missing::DisplayArea => "Откалибруйте айтрекер заново",
            },
            body: match reason {
                Missing::Calibration => format!(
                    "Tobii Eye Tracker 5 работает на калибровке, сделанной для другого \
                     человека{}, поэтому взгляд попадает не туда, куда вы смотрите. Калибровка \
                     займёт около минуты. Для отслеживания головы она не нужна.",
                    if area_built_in {
                        ", и с областью экрана не вашего монитора"
                    } else {
                        ""
                    }
                ),
                Missing::DisplayArea => format!(
                    "Ваша калибровка сделана без настройки экрана (с --no-display-setup или до \
                     tobii-linux 0.4.1), поэтому трекер соотносит взгляд с экраном {w_ru} × \
                     {h_ru} мм, а не с вашим. Повторная калибровка займёт около минуты."
                ),
            },
            calibrate: "Откалибровать",
            never: "Больше не напоминать",
            by_hand: "Откройте «Калибровка Tobii» среди приложений или запустите tobii-calibrate.",
        },
    }
}

/// What the user did with the notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Calibrate,
    Never,
    /// Closed, timed out, or shown without buttons.
    Nothing,
}

/// What `notify-send` prints for the action chosen.
fn answer(stdout: &str) -> Answer {
    match stdout.trim() {
        "calibrate" | "default" => Answer::Calibrate,
        "never" => Answer::Never,
        _ => Answer::Nothing,
    }
}

/// What the installed `notify-send` can do: buttons, and the activation
/// token that lets the calibration's window take the focus. `None` without
/// a `notify-send`.
fn notify_send_features() -> Option<(bool, bool)> {
    let help = Command::new("notify-send")
        .arg("--help")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let help = String::from_utf8_lossy(&help.stdout);
    Some((
        help.contains("--action"),
        help.contains("--activation-token-fd"),
    ))
}

/// Where `notify-send` leaves the activation token for the window: the
/// user's runtime directory, which only they can write to. None without
/// one: a predictable name in a shared /tmp could be someone else's link.
fn token_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|d| !d.is_empty())
        .map(|dir| PathBuf::from(dir).join(format!("tobii-calibrate-token-{}", std::process::id())))
}

/// Show the reminder and wait for what the user does with it, with the
/// activation token the notification server gave the button, if any.
fn notify(texts: &Texts) -> (Answer, Option<String>) {
    let by_hand = format!("{}\n\n{}", texts.body, texts.by_hand);
    match notify_send_features() {
        Some((true, with_token)) => notify_with_buttons(texts, with_token),
        Some((false, _)) => {
            // No desktop-entry hint: tied to the application, the
            // notification would go when notify-send exits, at once.
            let shown = Command::new("notify-send")
                .arg(format!("--app-name={APP_NAME}"))
                .args(["--icon=tobii-calibrate", "--", texts.summary, &by_hand])
                .stdin(Stdio::null())
                .status();
            report_shown("notify-send", shown);
            (Answer::Nothing, None)
        }
        None => {
            // busctl takes a negative number for an option without `--`.
            let shown = Command::new("busctl")
                .args([
                    "--user",
                    "call",
                    "--",
                    "org.freedesktop.Notifications",
                    "/org/freedesktop/Notifications",
                    "org.freedesktop.Notifications",
                    "Notify",
                    "susssasa{sv}i",
                    APP_NAME,
                    "0",
                    "tobii-calibrate",
                    texts.summary,
                    &by_hand,
                    "0",
                    "0",
                    "-1",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .status();
            report_shown("busctl", shown);
            (Answer::Nothing, None)
        }
    }
}

fn report_shown(how: &str, shown: io::Result<std::process::ExitStatus>) {
    match shown {
        Ok(status) if status.success() => tracing::info!(how, "reminder shown"),
        Ok(status) => tracing::warn!(how, %status, "the reminder could not be shown"),
        Err(e) => tracing::warn!(how, error = %e, "the reminder could not be shown"),
    }
}

/// The notification with its buttons; `notify-send` waits for the user and
/// prints the action they chose. With `with_token`, a shell gives it a file
/// to write the activation token to (it takes only a descriptor).
fn notify_with_buttons(texts: &Texts, with_token: bool) -> (Answer, Option<String>) {
    let args = [
        format!("--app-name={APP_NAME}"),
        "--icon=tobii-calibrate".to_owned(),
        format!("--hint=string:desktop-entry:{DESKTOP_ENTRY}"),
        // A click on the notification itself calibrates too.
        format!("--action=default={}", texts.calibrate),
        format!("--action=calibrate={}", texts.calibrate),
        format!("--action=never={}", texts.never),
        "--".to_owned(),
        texts.summary.to_owned(),
        texts.body.clone(),
    ];
    let token_file = with_token.then(token_path).flatten();
    let mut command = if let Some(file) = &token_file {
        let mut sh = Command::new("sh");
        sh.args([
            "-c",
            r#"file=$1; shift; exec notify-send --activation-token-fd=3 "$@" 3>"$file""#,
            "sh",
        ])
        .arg(file);
        sh
    } else {
        Command::new("notify-send")
    };
    tracing::info!("reminder shown, waiting for the user");
    let output = command
        .args(&args)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output();
    let token = token_file.and_then(|file| {
        let token = fs::read_to_string(&file).ok();
        let _ = fs::remove_file(&file);
        token.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
    });
    match output {
        Ok(out) if out.status.success() => {
            let chosen = answer(&String::from_utf8_lossy(&out.stdout));
            tracing::info!(?chosen, "the reminder was answered");
            (chosen, token)
        }
        Ok(out) => {
            tracing::warn!(status = %out.status, "the reminder could not be shown");
            (Answer::Nothing, None)
        }
        Err(e) => {
            tracing::warn!(error = %e, "the reminder could not be shown");
            (Answer::Nothing, None)
        }
    }
}

/// Run the calibration the user asked for, and wait for it: the autostart
/// unit's processes may be stopped with it once this exits.
fn calibrate(token: Option<&str>) -> Result<()> {
    // An upgrade since login leaves this program's file gone: the new one is
    // on the PATH.
    let program = std::env::current_exe()
        .ok()
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("tobii-calibrate"));
    let mut command = Command::new(program);
    command.stdin(Stdio::null());
    if let Some(token) = token {
        // winit reads the first on Wayland, the second on X11.
        command
            .env("XDG_ACTIVATION_TOKEN", token)
            .env("DESKTOP_STARTUP_ID", token);
    }
    let status = command
        .status()
        .context("could not start the calibration")?;
    tracing::info!(%status, "the calibration the reminder started has ended");
    Ok(())
}

/// Whether a session of this class gets reminders: a user's, not a
/// greeter's, a lock screen's or a background one. A session that does not
/// say is taken for a user's.
fn reminded_class(class: Option<&str>) -> bool {
    class.is_none_or(|c| matches!(c, "user" | "user-early"))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `--remind`.
///
/// # Errors
///
/// When what was reminded of cannot be saved, or the calibration it offers
/// cannot be started.
pub(crate) fn run() -> Result<()> {
    let class = std::env::var("XDG_SESSION_CLASS").ok();
    if !reminded_class(class.as_deref()) {
        tracing::info!(?class, "not a user's session: no reminder");
        return Ok(());
    }
    if std::env::var_os(store::ENV_OVERRIDE).is_some_and(|v| !v.is_empty()) {
        tracing::info!(
            "{} is set: the calibration is the user's business",
            store::ENV_OVERRIDE
        );
        return Ok(());
    }
    let Some(path) = state_path() else {
        tracing::info!("no per-user directory: no reminder");
        return Ok(());
    };
    if load(&path).off {
        tracing::debug!("reminders are off");
        return Ok(());
    }
    thread::sleep(SETTLE);
    let status = Status::read();
    if status.calibration_env.is_some() {
        tracing::info!(
            "tobiid gets {}: the calibration is the user's business",
            store::ENV_OVERRIDE
        );
        return Ok(());
    }
    let Some(missing) = status.missing() else {
        tracing::debug!("nothing to remind of");
        return Ok(());
    };
    if status.tracker.is_none() {
        tracing::info!(?missing, "the tracker is not plugged in: no reminder");
        return Ok(());
    }
    let mut reminders = load(&path);
    let now = now();
    if !reminders.due(missing, now) {
        tracing::debug!(?missing, "not due");
        return Ok(());
    }
    // Before it shows: a reminder cut short still counts.
    reminders.record(missing, now);
    save(&path, &reminders)?;
    let language = language(|name| std::env::var(name).ok());
    let area_built_in = matches!(status.area, Area::BuiltIn { .. } | Area::Unusable { .. })
        && status.display_env.is_none();
    match notify(&texts(missing, area_built_in, language)) {
        (Answer::Calibrate, token) => calibrate(token.as_deref()),
        (Answer::Never, _) => {
            reminders.off = true;
            save(&path, &reminders)
        }
        (Answer::Nothing, _) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_reason_is_reminded_of_three_times_a_day_then_a_week_apart() {
        let mut r = Reminders::default();
        let start = 1_000_000;
        let mut shown_at = Vec::new();
        for hour in 0..24 * 30 {
            let now = start + hour * 3600;
            if r.due(Missing::Calibration, now) {
                r.record(Missing::Calibration, now);
                shown_at.push((now - start) / 3600);
            }
        }

        assert_eq!(shown_at, [0, 24, 24 + 7 * 24]);
        assert!(
            r.due(Missing::DisplayArea, start),
            "each reason has its own"
        );
    }

    #[test]
    fn dont_remind_me_stops_every_reminder() {
        let r = Reminders {
            off: true,
            ..Reminders::default()
        };

        assert!(!r.due(Missing::Calibration, 0));
        assert!(!r.due(Missing::DisplayArea, u64::MAX));
    }

    #[test]
    fn what_was_shown_reads_back() {
        let r = Reminders {
            off: true,
            calibration: Shown {
                times: 2,
                last: 1_761_988_000,
            },
            display_area: Shown::default(),
        };

        assert_eq!(Reminders::from_text(&r.to_text()), r);
        assert_eq!(Reminders::from_text(""), Reminders::default());
        let odd = "calibration two 5\nsomething else\ndisplay-area 1 9\n";
        assert_eq!(
            Reminders::from_text(odd),
            Reminders {
                display_area: Shown { times: 1, last: 9 },
                ..Reminders::default()
            }
        );
    }

    #[test]
    fn the_language_is_gettexts() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| (*v).to_owned())
            }
        };

        assert_eq!(language(env(&[("LANG", "ru_RU.UTF-8")])), Language::Russian);
        assert_eq!(language(env(&[("LANG", "en_US.UTF-8")])), Language::English);
        assert_eq!(language(env(&[("LANG", "de_DE.UTF-8")])), Language::English);
        assert_eq!(
            language(env(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", "de:ru:en")])),
            Language::Russian
        );
        assert_eq!(
            language(env(&[("LC_ALL", "C"), ("LANGUAGE", "ru")])),
            Language::English,
            "C ignores LANGUAGE"
        );
        assert_eq!(
            language(env(&[
                ("LC_MESSAGES", "ru_UA.UTF-8"),
                ("LANG", "en_GB.UTF-8")
            ])),
            Language::Russian
        );
        assert_eq!(language(env(&[])), Language::English);
    }

    #[test]
    fn every_reminder_has_its_words() {
        for language in [Language::English, Language::Russian] {
            for reason in [Missing::Calibration, Missing::DisplayArea] {
                let t = texts(reason, true, language);
                assert!(
                    [t.summary, t.calibrate, t.never, t.by_hand]
                        .iter()
                        .all(|s| !s.is_empty())
                        && !t.body.is_empty(),
                    "{language:?} {reason:?}"
                );
            }
        }
        let area = texts(Missing::DisplayArea, true, Language::English).body;
        assert!(area.contains("633.6 × 334.3 mm"), "{area}");
        let area = texts(Missing::DisplayArea, true, Language::Russian).body;
        assert!(area.contains("633,6 × 334,3 мм"), "{area}");
        // A display area of the user's, but no calibration (after --reset).
        for language in [Language::English, Language::Russian] {
            let with = texts(Missing::Calibration, true, language).body;
            let without = texts(Missing::Calibration, false, language).body;
            assert!(with.len() > without.len(), "{language:?}: {without}");
            assert!(
                !without.contains("monitor") && !without.contains("монитор"),
                "{without}"
            );
        }
    }

    #[test]
    fn the_buttons_answer_by_their_names() {
        assert_eq!(answer("calibrate\n"), Answer::Calibrate);
        assert_eq!(answer("default\n"), Answer::Calibrate);
        assert_eq!(answer("never\n"), Answer::Never);
        assert_eq!(answer(""), Answer::Nothing);
    }

    #[test]
    fn a_greeters_session_is_not_reminded() {
        assert!(reminded_class(Some("user")));
        assert!(reminded_class(Some("user-early")));
        assert!(reminded_class(None));
        assert!(!reminded_class(Some("greeter")));
        assert!(!reminded_class(Some("lock-screen")));
        assert!(!reminded_class(Some("background")));
    }
}
