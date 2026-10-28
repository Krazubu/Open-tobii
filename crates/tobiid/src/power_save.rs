//! Power save in and out: commands 3000 and 3010, as the DLL's
//! `tracker_power_save_activate` and `tracker_power_save_deactivate`
//! (0x18019d730, 0x18019fbc0) send them for PRP property 2. Neither carries a
//! payload, and the answer carries none either: all it reports is the error
//! word.
//!
//! The ET5 carries both out, and announces each change with a notification,
//! 3020 when power save begins and 3030 when it ends (probe 2026-10-21). It is
//! those notifications, folded into the facts, that the `POWER_SAVE_ACTIVE`
//! state answers from (see [`crate::requests`]), not the request here: the
//! DLL's setter writes no cache, and only the notifications and a 1490 do.
//! Power save does not stop the sensor: gaze kept coming at full rate with it
//! on.
//!
//! Power save is one state for the device, as in the DLL: the last request
//! wins and any client may end it. The DLL has no notion of a client going
//! away, since it lives in one process; the daemon does, and when the client
//! whose request is in effect goes away it ends power save for it, rather
//! than leave the tracker in a mode nobody is left to know about. That holder
//! is kept across a re-initialisation of the device, unlike the pause's: the
//! pause is undone by every init replay, but the ET5 keeps power save through
//! a re-open, the device's teardown and a new init included (probe
//! 2026-10-21, twice in a row), so a departing holder still has to end it
//! with its 3010. The daemon never re-applies power save after a re-init, as
//! the DLL never does (each setter has one caller, and create and reconnect
//! only read the state back from the 1490). A daemon that stops ends the
//! power save a client holds, as the client's departure would (see
//! [`end_at_shutdown`]), where the DLL's process ending sends nothing and
//! leaves it on; only a daemon killed outright leaves the tracker in power
//! save with nobody holding it (probe 2026-10-28: a SIGTERM, a restart and
//! a SIGINT each left a fresh daemon's 1490 reading power save off, where a
//! SIGKILL control left it on).
//!
//! Requests hold `power_save_lock` from before the command to the holder's
//! update, so the device and the holder agree; the state lock is taken only
//! briefly and never across the command.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tobii_ipc::deadline;
use tobii_ipc::request::status;
use tobii_proto::protocol::cmd;
use tracing::{info, warn};

use crate::daemon::{State, lock_state, lock_until, look_for_tracker};
use crate::device::run;
use crate::requests::Reply;

/// How long the device may take to answer 3000 or 3010.
const POWER_SAVE_TIMEOUT: Duration = deadline::POWER_SAVE;
/// How long a request waits for another power-save request to finish.
#[cfg(not(test))]
const LOCK_TIMEOUT: Duration = deadline::POWER_SAVE_LOCK;
#[cfg(test)]
const LOCK_TIMEOUT: Duration = Duration::from_millis(300);

/// Answer a request to enter (`[1]`) or leave (`[0]`) power save from
/// `client`.
pub(crate) fn handle(state: &Mutex<State>, client: u64, payload: &[u8]) -> Reply {
    match payload {
        [1] => set(state, client, true),
        [0] => set(state, client, false),
        _ => Reply::err(status::INVALID_PARAMETER),
    }
}

/// The power-save lock, as the state holds it.
fn power_save_lock(state: &Mutex<State>) -> Arc<Mutex<()>> {
    Arc::clone(&lock_state(state).power_save_lock)
}

/// The command that enters or leaves power save.
const fn command_for(on: bool) -> u32 {
    if on {
        cmd::POWER_SAVE_ACTIVATE
    } else {
        cmd::POWER_SAVE_DEACTIVATE
    }
}

fn set(state: &Mutex<State>, client: u64, on: bool) -> Reply {
    let deadline = Instant::now() + LOCK_TIMEOUT;
    let lock = power_save_lock(state);
    let Some(_held) = lock_until(&lock, deadline) else {
        return Reply::err(status::TIMED_OUT);
    };
    let on_bus = look_for_tracker(state);
    let Some(device) = lock_state(state).device_for(client, on_bus) else {
        return Reply::err(status::CONNECTION_FAILED);
    };
    match run(
        device.as_ref(),
        command_for(on),
        Vec::new(),
        POWER_SAVE_TIMEOUT,
    ) {
        Ok(_) => {
            info!(client, on, "power save set");
            lock_state(state).power_save_holder = on.then_some(client);
            Reply::ok(Vec::new())
        }
        Err(code) => Reply::err(code),
    }
}

/// `client` went away: if its request to enter power save is in effect, end
/// power save. Its reader calls this before it unlists the client. Only the
/// running engine is asked; none is started for this. A 3010 answered 0
/// leaves the facts saying power save is off at once (see
/// [`State::note_power_save_ended`]): the engine may stop right after, before
/// the tracker's 3030 is read (probe 2026-10-21, where a client asking a
/// second later was told it was still on).
pub(crate) fn release(state: &Mutex<State>, client: u64) {
    // No request of the client's runs any more (its reader waits for its
    // request worker before calling this), so the holder cannot become
    // `client` after this check.
    if lock_state(state).power_save_holder != Some(client) {
        return;
    }
    end_held(state, Some(client), "its client disconnected");
}

/// The daemon is stopping: end the power save a client holds, as that
/// client's departure would, before the engine and its device go. The ET5
/// keeps power save through a re-open (probe 2026-10-21), so a daemon that
/// stopped without this would leave the tracker in it with nobody holding
/// it. The DLL, whose process ending sends nothing, leaves it on; the daemon
/// keeps its promise that power save ends with its holder instead. Only a
/// daemon killed outright still leaves it on. What it costs a stop, measured
/// on the tracker (probe 2026-10-28): the 3010 went out 45-129 ms after the
/// signal, within one poll of it, and was answered in 8-22 ms of its 6 s
/// deadline, so the daemon was gone 71-171 ms after the signal — far inside
/// the 45 s the shipped unit allows a stop.
///
/// Whether anybody holds it is read under `power_save_lock` and never ahead
/// of it (see [`end_held`]), unlike in [`release`], whose client has no
/// request left to run: a request entering power save holds that lock across
/// its 3000 and records itself the holder only once the tracker has
/// answered, so a stop that looked first would find no holder and leave the
/// tracker in the power save that 3000 was just beginning.
pub(crate) fn end_at_shutdown(state: &Mutex<State>) {
    end_held(state, None, "the daemon is stopping");
}

/// End the power save `holder` holds (`None`: whoever holds it), for `why`.
fn end_held(state: &Mutex<State>, holder: Option<u64>, why: &'static str) {
    let held = |st: &State| match holder {
        Some(client) => st.power_save_holder == Some(client),
        None => st.power_save_holder.is_some(),
    };
    let deadline = Instant::now() + LOCK_TIMEOUT;
    let lock = power_save_lock(state);
    let Some(_held) = lock_until(&lock, deadline) else {
        // Another request has held the lock all this while, stuck on the
        // device; a 3010 now could cross it. The holder is let go, though
        // nothing ends power save.
        let mut st = lock_state(state);
        warn!(
            ?holder,
            held_by = ?st.power_save_holder,
            why,
            "could not end power save in time, so it may stay on"
        );
        if held(&st) {
            st.power_save_holder = None;
        }
        return;
    };
    let (client, device) = {
        let mut st = lock_state(state);
        if !held(&st) {
            return;
        }
        (st.power_save_holder.take(), st.commands())
    };
    let Some(device) = device else {
        warn!(
            ?client,
            why, "power save not ended: no engine running, so it may stay on"
        );
        return;
    };
    info!(?client, why, "ending power save for its holder");
    match run(
        device.as_ref(),
        command_for(false),
        Vec::new(),
        POWER_SAVE_TIMEOUT,
    ) {
        Ok(_) => {
            let state_updated = lock_state(state).note_power_save_ended();
            info!(state_updated, "power save ended");
        }
        Err(code) => warn!(status = code, "could not end power save"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;

    use super::*;
    use crate::daemon::tests::state_with_client;
    use crate::device::DeviceCommands;
    use tobii_ipc::request::{Request, kind};
    use tobii_proto::facts::DeviceFacts;
    use tobii_proto::protocol::ttp_error;
    use tobii_usb::engine::{CommandError, CommandResponse, Sample};

    /// Records every command and answers each with `answer`. A `gate` set
    /// before a command makes `run` stop in it, so a test can hold a request
    /// on the device while another thread runs.
    struct Recording {
        sent: Mutex<Vec<(u32, Vec<u8>)>>,
        deadlines: Mutex<Vec<Duration>>,
        answer: Mutex<Result<CommandResponse, CommandError>>,
        gate: Mutex<Option<Gate>>,
    }

    /// Where a command waits: `run` announces itself on `entered`, then
    /// waits for `release`. It holds one command, the first to arrive.
    struct Gate {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }

    impl DeviceCommands for Recording {
        fn run(
            &self,
            cmd: u32,
            payload: Vec<u8>,
            timeout: Duration,
        ) -> Result<CommandResponse, CommandError> {
            self.sent.lock().expect("sent").push((cmd, payload));
            self.deadlines.lock().expect("deadlines").push(timeout);
            // Taken out of its lock, so the wait below holds nothing.
            let gate = self.gate.lock().expect("gate").take();
            if let Some(gate) = gate {
                let _ = gate.entered.send(());
                let _ = gate.release.recv();
            }
            self.answer.lock().expect("answer").clone()
        }
    }

    struct Setup {
        state: Mutex<State>,
        device: Arc<Recording>,
    }

    /// Clients 1 and 2, and a stand-in device that answers, so no engine
    /// starts.
    fn setup() -> Setup {
        let device = Arc::new(Recording {
            sent: Mutex::new(Vec::new()),
            deadlines: Mutex::new(Vec::new()),
            answer: Mutex::new(Ok(CommandResponse::ok(Vec::new()))),
            gate: Mutex::new(None),
        });
        let mut st = state_with_client(1);
        st.clients.append(&mut state_with_client(2).clients);
        st.fake_device = Some(device.clone());
        Setup {
            state: Mutex::new(st),
            device,
        }
    }

    /// A request from `client`, through the daemon's dispatcher.
    fn ask(s: &Setup, client: u64, payload: &[u8]) -> Reply {
        crate::requests::handle(
            &s.state,
            client,
            &Request {
                id: 1,
                kind: kind::POWER_SAVE,
                payload,
            },
        )
    }

    fn sent(s: &Setup) -> Vec<(u32, Vec<u8>)> {
        std::mem::take(&mut *s.device.sent.lock().expect("sent"))
    }

    fn holder(s: &Setup) -> Option<u64> {
        lock_state(&s.state).power_save_holder
    }

    #[test]
    fn entering_and_leaving_send_3000_and_3010_with_no_payload() {
        let s = setup();
        assert_eq!(ask(&s, 1, &[1]), Reply::ok(Vec::new()));
        assert_eq!(sent(&s), [(3000, Vec::new())]);
        assert_eq!(holder(&s), Some(1));
        assert_eq!(ask(&s, 1, &[0]), Reply::ok(Vec::new()));
        assert_eq!(sent(&s), [(3010, Vec::new())]);
        assert_eq!(holder(&s), None);
        assert!(
            s.device
                .deadlines
                .lock()
                .expect("deadlines")
                .iter()
                .all(|d| *d == deadline::POWER_SAVE),
            "every command waits the power-save deadline"
        );
    }

    #[test]
    fn a_request_waits_on_the_device_within_its_budget() {
        let s = setup();
        for payload in [[1], [0]] {
            s.device.deadlines.lock().expect("deadlines").clear();
            assert_eq!(ask(&s, 1, &payload), Reply::ok(Vec::new()));
            let sent = s.device.deadlines.lock().expect("deadlines").clone();
            // After waiting for another power-save request to finish.
            let waited = deadline::POWER_SAVE_LOCK + deadline::commands(&sent);
            assert!(
                !sent.is_empty() && waited <= deadline::worst::POWER_SAVE,
                "{payload:?}: {waited:?}"
            );
        }
    }

    #[test]
    fn the_last_request_wins_and_any_client_may_leave() {
        let s = setup();
        ask(&s, 1, &[1]);
        assert_eq!(holder(&s), Some(1));
        ask(&s, 2, &[1]);
        assert_eq!(holder(&s), Some(2), "the last request wins");
        ask(&s, 1, &[0]);
        assert_eq!(holder(&s), None, "any client may leave power save");
    }

    #[test]
    fn a_departing_holder_ends_power_save() {
        let s = setup();
        ask(&s, 1, &[1]);
        sent(&s);
        release(&s.state, 1);
        assert_eq!(sent(&s), [(3010, Vec::new())]);
        assert_eq!(holder(&s), None);
    }

    /// Facts whose 1490 reported power save as `value`.
    fn facts_saying(value: &str) -> Arc<DeviceFacts> {
        Arc::new(DeviceFacts {
            status: vec![(tobii_proto::facts::STATUS_POWER_SAVE, value.to_owned())],
            ..DeviceFacts::default()
        })
    }

    fn power_save_in_facts(s: &Setup) -> Option<String> {
        lock_state(&s.state).facts.as_ref().and_then(|f| {
            f.status_string(tobii_proto::facts::STATUS_POWER_SAVE)
                .map(str::to_owned)
        })
    }

    #[test]
    fn a_departing_holder_leaves_the_facts_saying_power_save_is_off() {
        // The engine may stop before the tracker's 3030 is read.
        let s = setup();
        ask(&s, 1, &[1]);
        lock_state(&s.state).facts = Some(facts_saying("true"));
        release(&s.state, 1);
        assert_eq!(power_save_in_facts(&s).as_deref(), Some("false"));
    }

    #[test]
    fn a_refused_3010_leaves_the_facts_as_they_were() {
        let s = setup();
        ask(&s, 1, &[1]);
        lock_state(&s.state).facts = Some(facts_saying("true"));
        *s.device.answer.lock().expect("answer") = Ok(CommandResponse {
            status: 1,
            error: ttp_error::NOT_SUPPORTED,
            payload: Vec::new(),
        });
        release(&s.state, 1);
        assert_eq!(power_save_in_facts(&s).as_deref(), Some("true"));
    }

    #[test]
    fn a_stopping_daemon_ends_the_power_save_a_client_holds() {
        let s = setup();
        ask(&s, 2, &[1]);
        lock_state(&s.state).facts = Some(facts_saying("true"));
        sent(&s);
        end_at_shutdown(&s.state);
        assert_eq!(sent(&s), [(3010, Vec::new())]);
        assert_eq!(holder(&s), None);
        assert_eq!(power_save_in_facts(&s).as_deref(), Some("false"));
    }

    #[test]
    fn a_stopping_daemon_with_no_holder_sends_nothing() {
        // Power save a client ended, or one nobody here entered, is left as
        // it is: the daemon ends only what it holds for a client.
        let s = setup();
        ask(&s, 1, &[1]);
        ask(&s, 1, &[0]);
        sent(&s);
        end_at_shutdown(&s.state);
        assert!(sent(&s).is_empty());
    }

    #[test]
    fn a_stopping_daemon_waits_for_a_request_that_is_entering_power_save() {
        let s = Arc::new(setup());
        lock_state(&s.state).facts = Some(facts_saying("true"));
        let (entered, reached_device) = mpsc::channel();
        let (release, held_there) = mpsc::channel();
        *s.device.gate.lock().expect("gate") = Some(Gate {
            entered,
            release: held_there,
        });

        // A client enters power save and its 3000 stops on the device, so
        // `power_save_lock` is held and the holder is not recorded yet.
        let asking = {
            let s = Arc::clone(&s);
            thread::spawn(move || ask(&s, 1, &[1]))
        };
        reached_device
            .recv_timeout(LOCK_TIMEOUT)
            .expect("the 3000 reaches the device");
        assert_eq!(holder(&s), None, "recorded only once the tracker answers");

        // The daemon stops while that 3000 is in flight. It may not read the
        // holder past the lock: there is none to find yet, and returning here
        // would leave the tracker in the power save the 3000 is beginning.
        let (done, stopped) = mpsc::channel();
        let stopping = {
            let s = Arc::clone(&s);
            thread::spawn(move || {
                end_at_shutdown(&s.state);
                let _ = done.send(());
            })
        };
        // Not finishing is weaker than it reads, since a thread yet to be
        // scheduled looks the same; what pins the behaviour is the pair of
        // assertions after the release, which no 3010 ever reaches without
        // the wait.
        assert!(
            stopped.recv_timeout(LOCK_TIMEOUT / 3).is_err(),
            "the stop waited for no power-save request still on the device"
        );

        // The 3000 is answered; the stop then finds the holder it recorded.
        release.send(()).expect("release the device");
        assert_eq!(asking.join().expect("the request"), Reply::ok(Vec::new()));
        stopping.join().expect("the stop");

        assert_eq!(sent(&s), [(3000, Vec::new()), (3010, Vec::new())]);
        assert_eq!(holder(&s), None);
        assert_eq!(power_save_in_facts(&s).as_deref(), Some("false"));
    }

    #[test]
    fn a_departing_client_that_does_not_hold_it_sends_nothing() {
        let s = setup();
        ask(&s, 1, &[1]);
        sent(&s);
        release(&s.state, 2);
        assert!(sent(&s).is_empty());
        assert_eq!(holder(&s), Some(1));
    }

    #[test]
    fn a_departing_holder_that_cannot_get_the_lock_is_forgotten() {
        let s = setup();
        ask(&s, 1, &[1]);
        sent(&s);
        let lock = power_save_lock(&s.state);
        let held = lock.lock().expect("power-save lock");
        release(&s.state, 1);
        drop(held);
        assert!(sent(&s).is_empty(), "no 3010 without the lock");
        assert_eq!(holder(&s), None, "the departed client holds nothing");
    }

    #[test]
    fn the_holder_outlives_a_reinit_so_its_departure_still_ends_power_save() {
        // The ET5 keeps power save through a re-open (probe 2026-10-21), so
        // a holder's departure after one still has to end it.
        let s = setup();
        ask(&s, 1, &[1]);
        sent(&s);
        lock_state(&s.state).observe(&Sample::DeviceReady(Arc::new(DeviceFacts::default())));
        assert_eq!(holder(&s), Some(1));
        release(&s.state, 1);
        assert_eq!(sent(&s), [(3010, Vec::new())]);
    }

    #[test]
    fn a_refusal_is_passed_on_and_holds_nothing() {
        let s = setup();
        *s.device.answer.lock().expect("answer") = Ok(CommandResponse {
            status: 1,
            error: ttp_error::NOT_SUPPORTED,
            payload: Vec::new(),
        });
        assert_eq!(ask(&s, 1, &[1]), Reply::err(status::NOT_SUPPORTED));
        assert_eq!(holder(&s), None);
    }

    #[test]
    fn a_payload_that_is_neither_is_refused_and_sends_nothing() {
        let s = setup();
        for payload in [&[][..], &[2], &[1, 0]] {
            assert_eq!(ask(&s, 1, payload), Reply::err(status::INVALID_PARAMETER));
        }
        assert!(sent(&s).is_empty());
    }
}
