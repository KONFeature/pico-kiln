//! Profile JSON → [`Profile`] — the parse the architecture moved to Core 0 (the
//! sole flash owner), so the platform-generic control loop never touches the
//! filesystem and receives a ready-to-run `parsed` profile over the channel.
//! Ports `kiln/profile.py::Profile.__init__` + `_calculate_duration` (the latter
//! reused verbatim through [`Profile::new`], which is golden-tested in
//! `kiln-core`).
//!
//! A small hand-rolled, panic-safe JSON reader rather than `serde`: it keeps this
//! crate's pure layer dependency-free (as `kiln-core` is) and is exhaustively
//! tested, including against adversarial upload bytes. It validates exactly where
//! the reference does — `name` and `steps` are required, `steps` non-empty, a
//! `ramp` needs `target_temp`, a `hold` needs `duration` — and ignores the
//! presentation-only `temp_units`/`description` (and any extra keys), matching
//! `dict.get` / unused fields.

use core::fmt::{self, Write};

use kiln_core::profile::{
    Profile, ProfileError, Step, StepIssue, StepKind, StepProblem, MAX_HOLD_DURATION_S,
    MAX_RAMP_RATE, MAX_STEPS, MIN_TARGET_TEMP,
};

/// Why parsing a profile failed. The reference raises and the web layer turns any
/// exception into a 400; these typed variants preserve the cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileJsonError {
    /// Malformed JSON (the reference's `json.loads` raising).
    Syntax,
    /// `json_data['name']` — required.
    MissingName,
    /// `"steps" not in json_data`.
    MissingSteps,
    /// `if not self.steps` — at least one step.
    EmptySteps,
    /// A step `type` was not ramp/hold/cooling (or the `type` key was absent).
    BadStepType,
    /// A `ramp` step without `target_temp` (`_calculate_duration` indexes it).
    MissingTargetTemp,
    /// A `hold` step without `duration` (`_calculate_duration` indexes it).
    MissingDuration,
    /// More than [`MAX_STEPS`] steps.
    TooManySteps,
    /// `temp_units` is not Celsius. The controller runs every number as °C, so
    /// a Fahrenheit profile (which the web editor can produce) would fire at
    /// the wrong temperatures, e.g. a 200 °F candling hold at 200 °C.
    UnsupportedUnits,
    /// A step fails the safety bounds ([`Profile::validate`]).
    Invalid(StepIssue),
}

impl From<ProfileError> for ProfileJsonError {
    fn from(e: ProfileError) -> Self {
        match e {
            ProfileError::NoSteps => ProfileJsonError::EmptySteps,
            ProfileError::TooManySteps => ProfileJsonError::TooManySteps,
        }
    }
}

/// Parse AND safety-check a profile for running: [`parse_profile`] plus
/// [`Profile::validate`] against the configured `max_temp`. Use this at every
/// entry point (upload, run, schedule, recovery).
pub fn parse_runnable(json: &str, max_temp: f32) -> Result<Profile, ProfileJsonError> {
    let p = parse_profile(json)?;
    p.validate(max_temp).map_err(ProfileJsonError::Invalid)?;
    Ok(p)
}

/// Validate the raw bytes of an uploaded profile file before it is committed
/// to flash (UTF-8, JSON shape, and the [`parse_runnable`] bounds).
pub fn check_upload(bytes: &[u8], max_temp: f32) -> Result<(), ProfileJsonError> {
    let text = core::str::from_utf8(bytes).map_err(|_| ProfileJsonError::Syntax)?;
    parse_runnable(text, max_temp).map(|_| ())
}

/// Human-readable reason (short, ASCII, no quotes — it is sent as an HTTP
/// error body behind an `Invalid profile: ` prefix).
pub fn write_error<W: Write>(w: &mut W, e: &ProfileJsonError, max_temp: f32) -> fmt::Result {
    match e {
        ProfileJsonError::Syntax => w.write_str("malformed JSON"),
        ProfileJsonError::MissingName => w.write_str("name is required"),
        ProfileJsonError::MissingSteps => w.write_str("steps is required"),
        ProfileJsonError::EmptySteps => w.write_str("needs at least one step"),
        ProfileJsonError::BadStepType => w.write_str("step type must be ramp, hold or cooling"),
        ProfileJsonError::MissingTargetTemp => w.write_str("a ramp step needs target_temp"),
        ProfileJsonError::MissingDuration => w.write_str("a hold step needs duration"),
        ProfileJsonError::TooManySteps => write!(w, "more than {MAX_STEPS} steps"),
        ProfileJsonError::UnsupportedUnits => {
            w.write_str("temp_units must be c (the controller runs in Celsius)")
        }
        ProfileJsonError::Invalid(issue) => {
            let kind = match issue.kind {
                StepKind::Ramp => "ramp",
                StepKind::Hold => "hold",
                StepKind::Cooling => "cooling",
            };
            write!(w, "step {} ({}): ", issue.step + 1, kind)?;
            match issue.problem {
                StepProblem::TargetOutOfRange => write!(
                    w,
                    "target_temp must be between {} and {}",
                    MIN_TARGET_TEMP, max_temp
                ),
                StepProblem::HoldWithoutTarget => w.write_str("target_temp is required"),
                StepProblem::RateOutOfRange => {
                    write!(w, "desired_rate must be above 0 and at most {}", MAX_RAMP_RATE)
                }
                StepProblem::MinRateOutOfRange => {
                    w.write_str("min_rate must be between 0 and desired_rate")
                }
                StepProblem::DurationOutOfRange => {
                    write!(w, "duration must be between 0 and {} s", MAX_HOLD_DURATION_S)
                }
            }
        }
    }
}

/// Parse a profile JSON document into a [`Profile`]. The profile's `name` is
/// validated for presence (as the reference requires) but not returned: the
/// Rust port carries the *filename* as the running profile's identity (see
/// `kiln_core::protocol`), supplied alongside this parsed profile.
///
/// Shape only (plus Celsius units); see [`parse_runnable`] for the safety
/// bounds.
pub fn parse_profile(json: &str) -> Result<Profile, ProfileJsonError> {
    let mut p = Reader::new(json);
    p.skip_ws();
    p.expect(b'{')?;

    let mut steps: [Step; MAX_STEPS] = [Step::default(); MAX_STEPS];
    let mut n_steps = 0usize;
    let mut have_name = false;
    let mut have_steps = false;

    if !p.try_consume(b'}') {
        loop {
            p.skip_ws();
            let key = p.parse_key()?;
            p.skip_ws();
            p.expect(b':')?;
            p.skip_ws();
            match key {
                "name" => {
                    // Must be a string, as `json_data['name']` is used as one.
                    p.parse_string()?;
                    have_name = true;
                }
                "steps" => {
                    n_steps = p.parse_steps(&mut steps)?;
                    have_steps = true;
                }
                // Rust-only: the reference ignores the units and fires the
                // numbers as °C. Absent means Celsius (the reference default).
                "temp_units" => {
                    if !matches!(p.parse_string()?, "c" | "C") {
                        return Err(ProfileJsonError::UnsupportedUnits);
                    }
                }
                _ => p.skip_value(0)?,
            }
            p.skip_ws();
            if p.try_consume(b',') {
                continue;
            }
            p.expect(b'}')?;
            break;
        }
    }

    if !have_name {
        return Err(ProfileJsonError::MissingName);
    }
    if !have_steps {
        return Err(ProfileJsonError::MissingSteps);
    }
    Ok(Profile::new(&steps[..n_steps])?)
}

/// A cursor over the JSON bytes. Every method is bounds-checked and returns
/// [`ProfileJsonError::Syntax`] rather than panicking, so arbitrary upload bytes
/// are safe input.
struct Reader<'a> {
    b: &'a [u8],
    s: &'a str,
    i: usize,
}

const MAX_DEPTH: u32 = 16;

impl<'a> Reader<'a> {
    fn new(s: &'a str) -> Self {
        Reader {
            b: s.as_bytes(),
            s,
            i: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), ProfileJsonError> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(ProfileJsonError::Syntax)
        }
    }

    fn try_consume(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    /// Parse a `"..."` string, returning the raw inner slice. Escapes are scanned
    /// past (so a closing quote is found correctly) but not decoded — profile
    /// keys and `type`/`name` values are plain text, and the slice is only ever
    /// compared, never re-emitted.
    /// An object key. Keys are matched undecoded, so an escaped spelling of a
    /// real key (`"temp_\u0075nits": "f"`) would be skipped as unknown and a
    /// Fahrenheit profile would run as Celsius: no real key needs an escape.
    fn parse_key(&mut self) -> Result<&'a str, ProfileJsonError> {
        let key = self.parse_string()?;
        if key.contains('\\') {
            return Err(ProfileJsonError::Syntax);
        }
        Ok(key)
    }

    fn parse_string(&mut self) -> Result<&'a str, ProfileJsonError> {
        self.expect(b'"')?;
        let start = self.i;
        while let Some(c) = self.peek() {
            match c {
                b'\\' => self.i += 2,
                b'"' => {
                    let slice = self.s.get(start..self.i).ok_or(ProfileJsonError::Syntax)?;
                    self.i += 1;
                    return Ok(slice);
                }
                _ => self.i += 1,
            }
        }
        Err(ProfileJsonError::Syntax)
    }

    /// Parse a JSON number into `f64`.
    fn parse_number(&mut self) -> Result<f64, ProfileJsonError> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.i += 1;
            } else {
                break;
            }
        }
        self.s
            .get(start..self.i)
            .and_then(|t| t.parse::<f64>().ok())
            .ok_or(ProfileJsonError::Syntax)
    }

    /// Parse the `steps` array into `out`, returning the count.
    fn parse_steps(&mut self, out: &mut [Step; MAX_STEPS]) -> Result<usize, ProfileJsonError> {
        self.skip_ws();
        self.expect(b'[')?;
        let mut n = 0usize;
        self.skip_ws();
        if self.try_consume(b']') {
            return Ok(0);
        }
        loop {
            if n >= MAX_STEPS {
                return Err(ProfileJsonError::TooManySteps);
            }
            self.skip_ws();
            out[n] = self.parse_step()?;
            n += 1;
            self.skip_ws();
            if self.try_consume(b',') {
                continue;
            }
            self.expect(b']')?;
            return Ok(n);
        }
    }

    /// Parse one step object into a typed [`Step`], applying the reference's
    /// per-type required-field rules.
    fn parse_step(&mut self) -> Result<Step, ProfileJsonError> {
        self.expect(b'{')?;
        let mut kind: Option<StepKind> = None;
        // `Step` stores f32; parse as f64 then narrow (kiln temps/rates/durations
        // are far inside f32's exact range).
        let mut target_temp: Option<f32> = None;
        let mut desired_rate: Option<f32> = None;
        let mut min_rate: Option<f32> = None;
        let mut duration: Option<f32> = None;

        self.skip_ws();
        if !self.try_consume(b'}') {
            loop {
                self.skip_ws();
                let key = self.parse_key()?;
                self.skip_ws();
                self.expect(b':')?;
                self.skip_ws();
                match key {
                    "type" => {
                        kind = Some(match self.parse_string()? {
                            "ramp" => StepKind::Ramp,
                            "hold" => StepKind::Hold,
                            "cooling" => StepKind::Cooling,
                            _ => return Err(ProfileJsonError::BadStepType),
                        });
                    }
                    "target_temp" => target_temp = Some(self.parse_number()? as f32),
                    "desired_rate" => desired_rate = Some(self.parse_number()? as f32),
                    "min_rate" => min_rate = Some(self.parse_number()? as f32),
                    "duration" => duration = Some(self.parse_number()? as f32),
                    _ => self.skip_value(0)?,
                }
                self.skip_ws();
                if self.try_consume(b',') {
                    continue;
                }
                self.expect(b'}')?;
                break;
            }
        }

        let kind = kind.ok_or(ProfileJsonError::BadStepType)?;
        Ok(match kind {
            StepKind::Ramp => Step {
                kind: StepKind::Ramp,
                target_temp: Some(target_temp.ok_or(ProfileJsonError::MissingTargetTemp)?),
                desired_rate,
                min_rate,
                duration: None,
            },
            StepKind::Hold => Step {
                kind: StepKind::Hold,
                target_temp,
                desired_rate: None,
                min_rate: None,
                duration: Some(duration.ok_or(ProfileJsonError::MissingDuration)?),
            },
            StepKind::Cooling => Step {
                kind: StepKind::Cooling,
                target_temp,
                desired_rate: None,
                min_rate: None,
                duration: None,
            },
        })
    }

    /// Skip any JSON value (for ignored keys), bounded in depth so adversarial
    /// nesting cannot overflow the stack.
    fn skip_value(&mut self, depth: u32) -> Result<(), ProfileJsonError> {
        if depth > MAX_DEPTH {
            return Err(ProfileJsonError::Syntax);
        }
        self.skip_ws();
        match self.peek().ok_or(ProfileJsonError::Syntax)? {
            b'"' => {
                self.parse_string()?;
                Ok(())
            }
            b'{' => self.skip_container(depth, b'}'),
            b'[' => self.skip_container(depth, b']'),
            b't' => self.skip_literal(b"true"),
            b'f' => self.skip_literal(b"false"),
            b'n' => self.skip_literal(b"null"),
            c if c.is_ascii_digit() || c == b'-' => {
                self.parse_number()?;
                Ok(())
            }
            _ => Err(ProfileJsonError::Syntax),
        }
    }

    fn skip_container(&mut self, depth: u32, close: u8) -> Result<(), ProfileJsonError> {
        self.i += 1; // opening bracket
        self.skip_ws();
        if self.try_consume(close) {
            return Ok(());
        }
        loop {
            self.skip_ws();
            if close == b'}' {
                self.parse_string()?;
                self.skip_ws();
                self.expect(b':')?;
            }
            self.skip_value(depth + 1)?;
            self.skip_ws();
            if self.try_consume(b',') {
                continue;
            }
            self.expect(close)?;
            return Ok(());
        }
    }

    fn skip_literal(&mut self, lit: &[u8]) -> Result<(), ProfileJsonError> {
        if self.b.get(self.i..self.i + lit.len()) == Some(lit) {
            self.i += lit.len();
            Ok(())
        } else {
            Err(ProfileJsonError::Syntax)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{
        "name": "Cone 6 Glaze",
        "temp_units": "c",
        "description": "bisque then glaze",
        "steps": [
            {"type": "ramp", "target_temp": 600, "desired_rate": 100, "min_rate": 80},
            {"type": "hold", "target_temp": 600, "duration": 600},
            {"type": "cooling", "target_temp": 100}
        ]
    }"#;

    #[test]
    fn parses_full_profile_with_correct_steps_and_duration() {
        let got = parse_profile(FULL).unwrap();
        // Reconstruct the same profile directly; duration is recomputed by
        // Profile::new (the golden-tested `_calculate_duration`).
        let expected = Profile::new(&[
            Step::ramp(600.0, Some(100.0), Some(80.0)),
            Step::hold(600.0, 600.0),
            Step::cooling(Some(100.0)),
        ])
        .unwrap();
        assert_eq!(got, expected);
        assert_eq!(got.step_count(), 3);
        // current_temp seeds from steps[0].target_temp (600), so the ramp-to-600
        // is 0s; hold 600s; cool 500°C@100/h = 18000s.
        assert_eq!(got.duration(), 0.0 + 600.0 + 18000.0);
    }

    #[test]
    fn ignores_unknown_and_presentation_fields() {
        let json = r#"{"extra": {"nested": [1,2,3]}, "name": "x", "bonus": true,
            "steps": [{"type":"hold","target_temp":120,"duration":60,"note":"hi"}]}"#;
        let got = parse_profile(json).unwrap();
        assert_eq!(got.step_count(), 1);
        assert_eq!(got.steps()[0].kind, StepKind::Hold);
        assert_eq!(got.steps()[0].duration, Some(60.0));
    }

    #[test]
    fn requires_name_and_steps() {
        assert_eq!(
            parse_profile(r#"{"steps":[{"type":"cooling"}]}"#),
            Err(ProfileJsonError::MissingName)
        );
        assert_eq!(
            parse_profile(r#"{"name":"x"}"#),
            Err(ProfileJsonError::MissingSteps)
        );
        assert_eq!(
            parse_profile(r#"{"name":"x","steps":[]}"#),
            Err(ProfileJsonError::EmptySteps)
        );
    }

    #[test]
    fn enforces_per_type_required_fields() {
        assert_eq!(
            parse_profile(r#"{"name":"x","steps":[{"type":"ramp","desired_rate":50}]}"#),
            Err(ProfileJsonError::MissingTargetTemp)
        );
        assert_eq!(
            parse_profile(r#"{"name":"x","steps":[{"type":"hold","target_temp":100}]}"#),
            Err(ProfileJsonError::MissingDuration)
        );
        assert_eq!(
            parse_profile(r#"{"name":"x","steps":[{"type":"bake","target_temp":100}]}"#),
            Err(ProfileJsonError::BadStepType)
        );
        assert_eq!(
            parse_profile(r#"{"name":"x","steps":[{"target_temp":100}]}"#),
            Err(ProfileJsonError::BadStepType)
        );
    }

    #[test]
    fn cooling_without_target_is_allowed() {
        let got = parse_profile(r#"{"name":"x","steps":[{"type":"cooling"}]}"#).unwrap();
        assert_eq!(got.steps()[0].kind, StepKind::Cooling);
        assert_eq!(got.steps()[0].target_temp, None);
    }

    #[test]
    fn rejects_too_many_steps() {
        // One more than MAX_STEPS cooling steps -> rejected.
        let mut s = String::from(r#"{"name":"x","steps":["#);
        for i in 0..(MAX_STEPS + 1) {
            if i > 0 {
                s.push(',');
            }
            s.push_str(r#"{"type":"cooling"}"#);
        }
        s.push_str("]}");
        assert_eq!(parse_profile(&s), Err(ProfileJsonError::TooManySteps));
    }

    #[test]
    fn malformed_json_is_a_syntax_error_not_a_panic() {
        for bad in [
            "",
            "{",
            "{\"name\"",
            "{\"name\":}",
            "{\"name\":\"x\",\"steps\":[",
            "{\"name\":\"x\",\"steps\":[{",
            "{\"name\":\"x\",\"steps\":[{\"type\":\"ramp\",\"target_temp\":}]}",
            "not json",
            "[]",
            "{\"steps\":[{\"type\":\"hold\",\"duration\":1,\"target_temp\":1}],\"name\":\"x\"\\",
        ] {
            // Never panics; always a typed Err.
            assert!(parse_profile(bad).is_err());
        }
    }

    #[test]
    fn shipped_profiles_are_runnable() {
        for (name, text) in [
            ("biscuit_faience", include_str!("../../../profiles/biscuit_faience.json")),
            ("test_ramp", include_str!("../../../profiles/test_ramp.json")),
        ] {
            assert!(parse_runnable(text, 1300.0).is_ok(), "{name}");
            assert!(check_upload(text.as_bytes(), 1300.0).is_ok(), "{name}");
        }
    }

    #[test]
    fn audit_h3_uploads_are_rejected() {
        let bad = |steps: &str| {
            parse_runnable(&format!(r#"{{"name":"x","steps":[{steps}]}}"#), 1300.0).unwrap_err()
        };
        let problem = |steps: &str| match bad(steps) {
            ProfileJsonError::Invalid(i) => i.problem,
            other => panic!("expected Invalid, got {other:?}"),
        };
        assert_eq!(
            problem(r#"{"type":"ramp","target_temp":600,"desired_rate":0}"#),
            StepProblem::RateOutOfRange
        );
        assert_eq!(
            problem(r#"{"type":"ramp","target_temp":600,"desired_rate":-100}"#),
            StepProblem::RateOutOfRange
        );
        assert_eq!(
            problem(r#"{"type":"ramp","target_temp":5000}"#),
            StepProblem::TargetOutOfRange
        );
        // 1e999 overflows f64 to +inf.
        assert_eq!(
            problem(r#"{"type":"ramp","target_temp":1e999}"#),
            StepProblem::TargetOutOfRange
        );
        assert_eq!(
            problem(r#"{"type":"hold","target_temp":600,"duration":1e999}"#),
            StepProblem::DurationOutOfRange
        );
        assert_eq!(
            problem(r#"{"type":"hold","duration":600}"#),
            StepProblem::HoldWithoutTarget
        );
        assert_eq!(
            problem(r#"{"type":"ramp","target_temp":600,"min_rate":-1}"#),
            StepProblem::MinRateOutOfRange
        );
        // Fahrenheit is refused outright; Celsius (either case) or absent is fine.
        let f = r#"{"name":"x","temp_units":"f","steps":[{"type":"cooling"}]}"#;
        assert_eq!(parse_profile(f), Err(ProfileJsonError::UnsupportedUnits));
        for units in [r#""c","#, r#""C","#, ""] {
            let json = if units.is_empty() {
                r#"{"name":"x","steps":[{"type":"cooling"}]}"#.to_string()
            } else {
                format!(r#"{{"name":"x","temp_units":{units}"steps":[{{"type":"cooling"}}]}}"#)
            };
            assert!(parse_runnable(&json, 1300.0).is_ok(), "{json}");
        }
        // An escaped key cannot smuggle Fahrenheit past the check.
        let sneaky = r#"{"name":"x","temp_\u0075nits":"f","steps":[{"type":"cooling"}]}"#;
        assert_eq!(parse_profile(sneaky), Err(ProfileJsonError::Syntax));
        // Non-UTF-8 upload bytes are a syntax error, not a panic.
        assert_eq!(check_upload(&[0xff, 0xfe], 1300.0), Err(ProfileJsonError::Syntax));
    }

    #[test]
    fn error_messages_fit_the_http_buffer() {
        let mut errs = vec![
            ProfileJsonError::Syntax,
            ProfileJsonError::MissingName,
            ProfileJsonError::MissingSteps,
            ProfileJsonError::EmptySteps,
            ProfileJsonError::BadStepType,
            ProfileJsonError::MissingTargetTemp,
            ProfileJsonError::MissingDuration,
            ProfileJsonError::TooManySteps,
            ProfileJsonError::UnsupportedUnits,
        ];
        for problem in [
            StepProblem::TargetOutOfRange,
            StepProblem::HoldWithoutTarget,
            StepProblem::RateOutOfRange,
            StepProblem::MinRateOutOfRange,
            StepProblem::DurationOutOfRange,
        ] {
            errs.push(ProfileJsonError::Invalid(StepIssue {
                step: 15,
                kind: StepKind::Cooling,
                problem,
            }));
        }
        for e in errs {
            let mut m = String::from("Invalid profile: ");
            write_error(&mut m, &e, 1400.0).unwrap();
            assert!(m.len() <= 96 && m.is_ascii() && !m.contains('"'), "{m}");
        }
        let mut m = String::new();
        write_error(
            &mut m,
            &ProfileJsonError::Invalid(StepIssue {
                step: 1,
                kind: StepKind::Hold,
                problem: StepProblem::DurationOutOfRange,
            }),
            1300.0,
        )
        .unwrap();
        assert_eq!(m, "step 2 (hold): duration must be between 0 and 259200 s");
    }

    #[test]
    fn deeply_nested_ignored_value_is_bounded() {
        // 64 nested arrays in an ignored field must error (depth bound), not
        // overflow the stack.
        let mut s = String::from(r#"{"name":"x","junk":"#);
        for _ in 0..64 {
            s.push('[');
        }
        for _ in 0..64 {
            s.push(']');
        }
        s.push_str(r#","steps":[{"type":"cooling"}]}"#);
        // Either parses (if within bound) or errors — must not crash.
        let _ = parse_profile(&s);
    }
}
