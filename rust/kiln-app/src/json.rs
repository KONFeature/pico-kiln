//! `Status` → JSON, the web boundary's exactness-critical surface. Replaces
//! `comms.StatusMessage.build` / `build_tuning_status` + `json.dumps` (which the
//! reference pre-encodes once per update in `status_receiver.get_status_json`).
//!
//! Two shapes, branching on state exactly as `control_thread.send_status_update`
//! does: the normal status and the `KilnState::Tuning` status (with its nested
//! `tuning` object). The field *names*, *order*, and *null/empty* conventions are
//! reproduced verbatim from the reference templates so the static front-end —
//! which reads each field by name and re-formats every number through
//! `toFixed()` — sees identical input. Numbers are emitted at the reference's
//! `round()` precision (2 dp for temps/SSR, 1 dp for elapsed/rate); because the
//! front-end reparses, fixed-decimal output is functionally identical to Python's
//! float `repr`. No allocation, no serde: writes straight into any
//! [`core::fmt::Write`], so picoserve can stream the same pre-formatted bytes.

use core::fmt::{self, Write};
use kiln_core::profile::StepKind;
use kiln_core::protocol::{ScheduledSnapshot, Status, TuningSnapshot};
use kiln_core::state::{KilnError, KilnState};
use kiln_core::tuner::TuningStage;

use crate::errors::write_error;
use crate::timefmt::write_iso;
use crate::tuning_names::{mode_str, step_name as tuning_step_name};

pub(crate) fn state_str(s: KilnState) -> &'static str {
    match s {
        KilnState::Idle => "IDLE",
        KilnState::Running => "RUNNING",
        KilnState::Tuning => "TUNING",
        KilnState::Complete => "COMPLETE",
        KilnState::Error => "ERROR",
    }
}

pub(crate) fn step_kind_str(k: StepKind) -> &'static str {
    match k {
        StepKind::Ramp => "ramp",
        StepKind::Hold => "hold",
        StepKind::Cooling => "cooling",
    }
}

fn stage_str(s: TuningStage) -> &'static str {
    match s {
        TuningStage::Running => "running",
        TuningStage::Complete => "complete",
        TuningStage::Error => "error",
    }
}

/// A JSON string literal with the minimal escaping JSON requires. The domain
/// values (validated filenames, static labels, reconstructed error messages)
/// never contain quotes or controls, but escaping keeps the output well-formed
/// regardless.
fn write_json_str<W: Write>(w: &mut W, s: &str) -> fmt::Result {
    w.write_char('"')?;
    for c in s.chars() {
        match c {
            '"' => w.write_str("\\\"")?,
            '\\' => w.write_str("\\\\")?,
            c if (c as u32) < 0x20 => write!(w, "\\u{:04x}", c as u32)?,
            c => w.write_char(c)?,
        }
    }
    w.write_char('"')
}

/// `error` as a JSON value: the reconstructed message string or `null`. Emitted
/// under two keys (`error` and `error_message`, see [`write_normal`]).
fn write_error_value<W: Write>(w: &mut W, error: &Option<KilnError>) -> fmt::Result {
    match error {
        Some(e) => {
            w.write_char('"')?;
            write_error(w, e)?;
            w.write_char('"')
        }
        None => w.write_str("null"),
    }
}

fn write_scheduled<W: Write>(w: &mut W, sc: &ScheduledSnapshot) -> fmt::Result {
    w.write_str("{\"profile_filename\":")?;
    write_json_str(w, sc.profile.as_str())?;
    write!(w, ",\"start_time\":{},\"start_time_iso\":\"", sc.start_time)?;
    write_iso(w, sc.start_time as i64)?;
    write!(w, "\",\"seconds_until_start\":{}}}", sc.seconds_until_start)
}

fn write_normal<W: Write>(w: &mut W, s: &Status) -> fmt::Result {
    write!(w, "{{\"timestamp\":{},\"state\":", s.timestamp)?;
    write_json_str(w, state_str(s.state))?;
    write!(
        w,
        ",\"current_temp\":{:.2},\"target_temp\":{:.2},\"ssr_output\":{:.2},\"elapsed\":{:.1},\"profile_name\":",
        s.current_temp, s.target_temp, s.ssr_output, s.elapsed
    )?;
    match &s.profile_name {
        Some(p) => write_json_str(w, p.as_str())?,
        None => w.write_str("null")?,
    }
    // Emitted under both keys: the reference / on-device static pages read
    // `error`; the React web app reads `error_message`. Same value for both.
    w.write_str(",\"error\":")?;
    write_error_value(w, &s.error)?;
    w.write_str(",\"error_message\":")?;
    write_error_value(w, &s.error)?;
    w.write_str(",\"step_index\":")?;
    match s.step_index {
        Some(i) => write!(w, "{}", i)?,
        None => w.write_str("null")?,
    }
    // step_name: null with no active profile, "" past the last step, else the
    // step type — the three branches of `build`.
    w.write_str(",\"step_name\":")?;
    match (s.step_index, s.step_kind) {
        (None, _) => w.write_str("null")?,
        (Some(_), None) => w.write_str("\"\"")?,
        (Some(_), Some(k)) => write_json_str(w, step_kind_str(k))?,
    }
    w.write_str(",\"total_steps\":")?;
    match s.total_steps {
        Some(n) => write!(w, "{}", n)?,
        None => w.write_str("null")?,
    }
    write!(
        w,
        ",\"desired_rate\":{:.1},\"step_elapsed\":{:.1},\"is_recovering\":{},\"recovery_target_temp\":",
        s.desired_rate,
        s.step_elapsed,
        s.is_recovering
    )?;
    match s.recovery_target_temp {
        Some(t) => write!(w, "{:.2}", t)?,
        None => w.write_str("null")?,
    }
    write!(
        w,
        ",\"stall_advances\":{},\"measured_rate\":{:.1},\"scheduled_profile\":",
        s.stall_advances,
        s.measured_rate
    )?;
    match &s.scheduled {
        Some(sc) => write_scheduled(w, sc)?,
        None => w.write_str("null")?,
    }
    w.write_char('}')
}

fn write_tuning_obj<W: Write>(w: &mut W, t: &TuningSnapshot) -> fmt::Result {
    w.write_str("{\"stage\":")?;
    write_json_str(w, stage_str(t.stage))?;
    w.write_str(",\"mode\":")?;
    write_json_str(w, mode_str(t.mode))?;
    // `elapsed` is the *step* elapsed: tuner.get_status() merges the step status,
    // whose `elapsed` overwrites the tuner total. `error` is null while the
    // builder runs (only reached during a live TUNING tick; faults surface via
    // the normal status path once state flips to ERROR).
    write!(
        w,
        ",\"max_temp\":{:.1},\"elapsed\":{:.1},\"step_index\":{},\"total_steps\":{},\"error\":null,\"step_name\":",
        t.max_temp, t.step_elapsed, t.step_index, t.total_steps
    )?;
    write_json_str(w, tuning_step_name(t.mode, t.step_index))?;
    write!(w, ",\"ssr_percent\":{:.1},\"target_temp\":", t.ssr_percent)?;
    match t.target_temp {
        Some(v) => write!(w, "{:.1}", v)?,
        None => w.write_str("null")?,
    }
    write!(
        w,
        ",\"timeout\":{:.1},\"plateau_detected\":{},\"peak_temp\":",
        t.timeout, t.plateau_detected
    )?;
    // `round(peak, 1) if peak else None`: 0.0 is falsy in the reference.
    if t.peak_temp == 0.0 {
        w.write_str("null")?;
    } else {
        write!(w, "{:.1}", t.peak_temp)?;
    }
    w.write_char('}')
}

fn write_tuning<W: Write>(w: &mut W, s: &Status, t: &TuningSnapshot) -> fmt::Result {
    write!(w, "{{\"timestamp\":{},\"state\":", s.timestamp)?;
    write_json_str(w, state_str(s.state))?;
    write!(
        w,
        ",\"current_temp\":{:.2},\"target_temp\":{:.2},\"elapsed\":{:.1},\"ssr_output\":{:.2},\"profile_name\":null,\"tuning\":",
        s.current_temp, s.target_temp, t.step_elapsed, s.ssr_output
    )?;
    write_tuning_obj(w, t)?;
    w.write_str(",\"step_name\":")?;
    write_json_str(w, tuning_step_name(t.mode, t.step_index))?;
    write!(
        w,
        ",\"step_index\":{},\"total_steps\":{}}}",
        t.step_index, t.total_steps
    )
}

/// Write `s` as the web status JSON, choosing the tuning shape exactly when the
/// reference would (`state == TUNING` with a live tuner snapshot).
pub fn write_status_json<W: Write>(w: &mut W, s: &Status) -> fmt::Result {
    match (s.state, &s.tuning) {
        (KilnState::Tuning, Some(t)) => write_tuning(w, s, t),
        _ => write_normal(w, s),
    }
}

/// Write the `GET /api/scheduled` body. Distinct from the embedded
/// `scheduled_profile`: keyed `profile` (not `profile_filename`) under a
/// `scheduled` bool (`handle_api_scheduled_status`).
pub fn write_scheduled_endpoint<W: Write>(w: &mut W, s: &Status) -> fmt::Result {
    match &s.scheduled {
        None => w.write_str("{\"scheduled\":false}"),
        Some(sc) => {
            w.write_str("{\"scheduled\":true,\"profile\":")?;
            write_json_str(w, sc.profile.as_str())?;
            write!(w, ",\"start_time\":{},\"start_time_iso\":\"", sc.start_time)?;
            write_iso(w, sc.start_time as i64)?;
            write!(w, "\",\"seconds_until_start\":{}}}", sc.seconds_until_start)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_core::protocol::ProfileName;
    use kiln_core::tuner::TuningMode;

    fn render(s: &Status) -> String {
        let mut out = String::new();
        write_status_json(&mut out, s).unwrap();
        out
    }

    #[test]
    fn idle_status_matches_template() {
        let got = render(&Status::idle());
        assert_eq!(
            got,
            "{\"timestamp\":0,\"state\":\"IDLE\",\"current_temp\":0.00,\"target_temp\":0.00,\
\"ssr_output\":0.00,\"elapsed\":0.0,\"profile_name\":null,\"error\":null,\"error_message\":null,\
\"step_index\":null,\
\"step_name\":null,\"total_steps\":null,\"desired_rate\":0.0,\"step_elapsed\":0.0,\
\"is_recovering\":false,\"recovery_target_temp\":null,\"stall_advances\":0,\"measured_rate\":0.0,\
\"scheduled_profile\":null}"
        );
    }

    #[test]
    fn running_ramp_with_scheduled_profile() {
        let s = Status {
            timestamp: 1_700_000_000,
            state: KilnState::Running,
            current_temp: 123.46,
            target_temp: 200.0,
            ssr_output: 75.5,
            elapsed: 65.2,
            step_index: Some(1),
            step_kind: Some(StepKind::Ramp),
            total_steps: Some(3),
            desired_rate: 120.0,
            step_elapsed: 12.5,
            measured_rate: 95.0,
            profile_name: Some(ProfileName::new("cone6.json").unwrap()),
            scheduled: Some(ScheduledSnapshot {
                profile: ProfileName::new("bisque.json").unwrap(),
                start_time: 1_700_000_000,
                seconds_until_start: 3600,
            }),
            ..Status::idle()
        };
        assert_eq!(
            render(&s),
            "{\"timestamp\":1700000000,\"state\":\"RUNNING\",\"current_temp\":123.46,\
\"target_temp\":200.00,\"ssr_output\":75.50,\"elapsed\":65.2,\"profile_name\":\"cone6.json\",\
\"error\":null,\"error_message\":null,\"step_index\":1,\"step_name\":\"ramp\",\"total_steps\":3,\
\"desired_rate\":120.0,\
\"step_elapsed\":12.5,\"is_recovering\":false,\"recovery_target_temp\":null,\"stall_advances\":0,\"measured_rate\":95.0,\
\"scheduled_profile\":{\"profile_filename\":\"bisque.json\",\"start_time\":1700000000,\
\"start_time_iso\":\"2023-11-14 22:13:20\",\"seconds_until_start\":3600}}"
        );
    }

    #[test]
    fn active_profile_past_last_step_emits_empty_step_name() {
        // step_index set but no current step → step_name "" (not null).
        let s = Status {
            state: KilnState::Running,
            step_index: Some(3),
            step_kind: None,
            total_steps: Some(3),
            ..Status::idle()
        };
        let got = render(&s);
        assert!(got.contains("\"step_index\":3,\"step_name\":\"\",\"total_steps\":3"));
    }

    #[test]
    fn error_status_reconstructs_message() {
        let s = Status {
            state: KilnState::Error,
            error: Some(kiln_core::state::KilnError::NoActiveProfile),
            ..Status::idle()
        };
        let got = render(&s);
        assert!(got.contains("\"state\":\"ERROR\""));
        assert!(got.contains("\"error\":\"No active profile\""));
        // The React web app reads `error_message`; it must carry the same text.
        assert!(got.contains("\"error_message\":\"No active profile\""));
    }

    #[test]
    fn tuning_status_matches_template() {
        let s = Status {
            timestamp: 1_700_000_000,
            state: KilnState::Tuning,
            current_temp: 305.25,
            target_temp: 0.0,
            ssr_output: 50.0,
            tuning: Some(TuningSnapshot {
                stage: TuningStage::Running,
                mode: TuningMode::Standard,
                max_temp: 150.0,
                step_index: 2,
                total_steps: 6,
                step_elapsed: 42.5,
                ssr_percent: 50.0,
                target_temp: None,
                timeout: 1800.0,
                plateau_detected: false,
                peak_temp: 310.0,
            }),
            ..Status::idle()
        };
        assert_eq!(
            render(&s),
            "{\"timestamp\":1700000000,\"state\":\"TUNING\",\"current_temp\":305.25,\
\"target_temp\":0.00,\"elapsed\":42.5,\"ssr_output\":50.00,\"profile_name\":null,\
\"tuning\":{\"stage\":\"running\",\"mode\":\"STANDARD\",\"max_temp\":150.0,\"elapsed\":42.5,\
\"step_index\":2,\"total_steps\":6,\"error\":null,\"step_name\":\"heat_50pct_plateau\",\
\"ssr_percent\":50.0,\"target_temp\":null,\"timeout\":1800.0,\"plateau_detected\":false,\
\"peak_temp\":310.0},\"step_name\":\"heat_50pct_plateau\",\"step_index\":2,\"total_steps\":6}"
        );
    }

    #[test]
    fn tuning_peak_temp_zero_is_null() {
        let s = Status {
            state: KilnState::Tuning,
            tuning: Some(TuningSnapshot {
                stage: TuningStage::Running,
                mode: TuningMode::Safe,
                max_temp: 100.0,
                step_index: 0,
                total_steps: 3,
                step_elapsed: 1.0,
                ssr_percent: 60.0,
                target_temp: Some(100.0),
                timeout: 600.0,
                plateau_detected: false,
                peak_temp: 0.0,
            }),
            ..Status::idle()
        };
        let got = render(&s);
        assert!(got.contains("\"peak_temp\":null"));
        assert!(got.contains("\"target_temp\":100.0"));
        assert!(got.contains("\"step_name\":\"heat_60pct_to_100C\""));
    }

    #[test]
    fn scheduled_endpoint_both_shapes() {
        let mut none = String::new();
        write_scheduled_endpoint(&mut none, &Status::idle()).unwrap();
        assert_eq!(none, "{\"scheduled\":false}");

        let s = Status {
            scheduled: Some(ScheduledSnapshot {
                profile: ProfileName::new("bisque.json").unwrap(),
                start_time: 1_700_000_000,
                seconds_until_start: 3600,
            }),
            ..Status::idle()
        };
        let mut some = String::new();
        write_scheduled_endpoint(&mut some, &s).unwrap();
        assert_eq!(
            some,
            "{\"scheduled\":true,\"profile\":\"bisque.json\",\"start_time\":1700000000,\
\"start_time_iso\":\"2023-11-14 22:13:20\",\"seconds_until_start\":3600}"
        );
    }
}
