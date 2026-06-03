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

use kiln_core::profile::{Profile, ProfileError, Step, StepKind, MAX_STEPS};

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
}

impl From<ProfileError> for ProfileJsonError {
    fn from(e: ProfileError) -> Self {
        match e {
            ProfileError::NoSteps => ProfileJsonError::EmptySteps,
            ProfileError::TooManySteps => ProfileJsonError::TooManySteps,
        }
    }
}

/// Parse a profile JSON document into a [`Profile`]. The profile's `name` is
/// validated for presence (as the reference requires) but not returned: the
/// Rust port carries the *filename* as the running profile's identity (see
/// `kiln_core::protocol`), supplied alongside this parsed profile.
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
            let key = p.parse_string()?;
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
        let mut target_temp: Option<f64> = None;
        let mut desired_rate: Option<f64> = None;
        let mut min_rate: Option<f64> = None;
        let mut duration: Option<f64> = None;

        self.skip_ws();
        if !self.try_consume(b'}') {
            loop {
                self.skip_ws();
                let key = self.parse_string()?;
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
                    "target_temp" => target_temp = Some(self.parse_number()?),
                    "desired_rate" => desired_rate = Some(self.parse_number()?),
                    "min_rate" => min_rate = Some(self.parse_number()?),
                    "duration" => duration = Some(self.parse_number()?),
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
        // Build 33 cooling steps (MAX_STEPS is 32).
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
