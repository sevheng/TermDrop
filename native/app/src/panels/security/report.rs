//! The pure half of the Security panel: the audit's state machine and every
//! label `SecurityPanel.vue` computes from a report. No GPUI here, so all of
//! it is unit tested.

use termdrop_core::security::{SecurityCheck, SecurityReport};

/// One check's outcome. The core only ever emits the four literals; anything
/// else is shown the way `unknown` is, as Vue's `statusMeta` fallback does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pass,
    Warn,
    Fail,
    /// A probe that could not run (no privileges, tool missing). Excluded
    /// from the score by the core rather than counted as a failure.
    Unknown,
}

impl Status {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "pass" => Self::Pass,
            "warn" => Self::Warn,
            "fail" => Self::Fail,
            _ => Self::Unknown,
        }
    }

    pub fn tone(self) -> Tone {
        match self {
            Self::Pass => Tone::Good,
            Self::Warn => Tone::Warn,
            Self::Fail => Tone::Bad,
            Self::Unknown => Tone::Muted,
        }
    }
}

/// Which token family a mark is drawn in; the view maps it onto the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warn,
    Bad,
    /// `bg-input text-ink-2`: nothing to grade.
    Muted,
}

/// The badge beside a check's name: its status as sent, uppercased (Vue
/// uppercases it in CSS; GPUI has no text-transform).
pub fn badge_text(raw_status: &str) -> String {
    raw_status.to_uppercase()
}

/// The score circle's grade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    Good,
    Fair,
    Poor,
    /// Every check came back undetermined, so there is nothing to grade.
    Unknown,
}

impl Grade {
    pub fn of(report: &SecurityReport) -> Self {
        if !has_score(report) {
            return Self::Unknown;
        }
        match report.score {
            80.. => Self::Good,
            50.. => Self::Fair,
            _ => Self::Poor,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Good => "Good",
            Self::Fair => "Fair",
            Self::Poor => "Poor",
            Self::Unknown => "Unknown",
        }
    }

    pub fn tone(self) -> Tone {
        match self {
            Self::Good => Tone::Good,
            Self::Fair => Tone::Warn,
            Self::Poor => Tone::Bad,
            Self::Unknown => Tone::Muted,
        }
    }
}

/// False when every check came back undetermined.
pub fn has_score(report: &SecurityReport) -> bool {
    report.scored > 0
}

/// What the circle says: the score, or an em dash when there is none.
pub fn score_text(report: &SecurityReport) -> String {
    if has_score(report) {
        report.score.to_string()
    } else {
        "\u{2014}".into()
    }
}

/// The line under the grade.
pub fn summary(report: &SecurityReport) -> String {
    if has_score(report) {
        format!("{} of {} checks passed", report.passed, report.scored)
    } else {
        "Nothing could be determined".into()
    }
}

/// "2 failed · 1 warning · 3 undetermined", omitting whichever counts are
/// zero; empty when there is nothing to say.
pub fn breakdown(checks: &[SecurityCheck]) -> String {
    let count = |s: &str| checks.iter().filter(|c| c.status == s).count();
    let failed = count("fail");
    let warned = count("warn");
    let unknown = count("unknown");
    let mut parts = Vec::new();
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if warned > 0 {
        let s = if warned == 1 { "" } else { "s" };
        parts.push(format!("{warned} warning{s}"));
    }
    if unknown > 0 {
        parts.push(format!("{unknown} undetermined"));
    }
    parts.join(" \u{b7} ")
}

/// The header's "updated 5m ago", from the audit's own timestamp (both the
/// core and Vue's store cache the report, so when the panel read it says
/// nothing). Empty when the report carries no time.
pub fn updated_label(generated_at: u64, now: u64) -> String {
    if generated_at == 0 {
        return String::new();
    }
    format!(
        "updated {}",
        super::super::common::format_elapsed(now.saturating_sub(generated_at))
    )
}

/// The header's "12 checks"; empty for a report with none.
pub fn checks_label(report: &SecurityReport) -> String {
    match report.checks.len() {
        0 => String::new(),
        n => format!("{n} checks"),
    }
}

/// Where the audit is. Mirrors one entry of the Vue store's
/// `securityReports`: starting a run clears the previous report, so a
/// refresh shows the loading state, as `setSecurityLoading` did.
#[derive(Debug, Clone, Default)]
pub enum AuditState {
    /// Nothing has run. Only seen if the lazy start was skipped.
    #[default]
    Idle,
    Loading,
    Failed(String),
    Ready(SecurityReport),
}

/// The audit plus a run counter, so that only the newest run's answer is
/// applied.
#[derive(Debug, Default)]
pub struct Audit {
    pub state: AuditState,
    run: u64,
}

impl Audit {
    /// A run is starting: returns its number for [`Self::finish`].
    pub fn begin(&mut self) -> u64 {
        self.run += 1;
        self.state = AuditState::Loading;
        self.run
    }

    /// Apply run `run`'s result. Returns false (and changes nothing) for an
    /// answer a newer run has superseded.
    pub fn finish(&mut self, run: u64, result: Result<SecurityReport, String>) -> bool {
        if run != self.run {
            return false;
        }
        self.state = match result {
            Ok(report) => AuditState::Ready(report),
            Err(e) => AuditState::Failed(e),
        };
        true
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.state, AuditState::Loading)
    }

    pub fn report(&self) -> Option<&SecurityReport> {
        match &self.state {
            AuditState::Ready(r) => Some(r),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use termdrop_core::security::Remediation;

    fn check(name: &str, status: &str) -> SecurityCheck {
        SecurityCheck {
            name: name.into(),
            status: status.into(),
            message: String::new(),
            detail: None,
            remediation: None,
        }
    }

    fn report(score: u8, passed: u16, scored: u16, checks: Vec<SecurityCheck>) -> SecurityReport {
        SecurityReport {
            generated_at: 1_000,
            score,
            passed,
            scored,
            checks,
        }
    }

    #[test]
    fn statuses_parse_and_anything_else_reads_as_unknown() {
        assert_eq!(Status::parse("pass"), Status::Pass);
        assert_eq!(Status::parse("warn"), Status::Warn);
        assert_eq!(Status::parse("fail"), Status::Fail);
        assert_eq!(Status::parse("unknown"), Status::Unknown);
        assert_eq!(Status::parse("PASS"), Status::Unknown);
        assert_eq!(Status::parse(""), Status::Unknown);
        assert_eq!(Status::Fail.tone(), Tone::Bad);
        assert_eq!(Status::Unknown.tone(), Tone::Muted);
        assert_eq!(badge_text("warn"), "WARN");
    }

    #[test]
    fn grades_follow_the_vue_thresholds() {
        let g = |score| Grade::of(&report(score, 1, 1, vec![]));
        assert_eq!(g(100), Grade::Good);
        assert_eq!(g(80), Grade::Good);
        assert_eq!(g(79), Grade::Fair);
        assert_eq!(g(50), Grade::Fair);
        assert_eq!(g(49), Grade::Poor);
        assert_eq!(g(0), Grade::Poor);
        assert_eq!(Grade::Fair.label(), "Fair");
        assert_eq!(Grade::Fair.tone(), Tone::Warn);
    }

    #[test]
    fn a_report_with_nothing_determined_has_no_score() {
        let r = report(0, 0, 0, vec![check("a", "unknown")]);
        assert!(!has_score(&r));
        assert_eq!(Grade::of(&r), Grade::Unknown);
        assert_eq!(Grade::of(&r).label(), "Unknown");
        assert_eq!(score_text(&r), "\u{2014}");
        assert_eq!(summary(&r), "Nothing could be determined");

        let r = report(67, 2, 3, vec![]);
        assert_eq!(score_text(&r), "67");
        assert_eq!(summary(&r), "2 of 3 checks passed");
    }

    #[test]
    fn breakdown_omits_zero_counts_and_pluralises_warnings() {
        assert_eq!(breakdown(&[]), "");
        assert_eq!(breakdown(&[check("a", "pass")]), "");
        assert_eq!(
            breakdown(&[
                check("a", "fail"),
                check("b", "fail"),
                check("c", "warn"),
                check("d", "unknown"),
                check("e", "unknown"),
                check("f", "unknown"),
                check("g", "pass"),
            ]),
            "2 failed \u{b7} 1 warning \u{b7} 3 undetermined"
        );
        assert_eq!(
            breakdown(&[check("a", "warn"), check("b", "warn")]),
            "2 warnings"
        );
    }

    #[test]
    fn header_labels() {
        assert_eq!(updated_label(1_000, 1_002), "updated just now");
        assert_eq!(updated_label(1_000, 1_300), "updated 5m ago");
        // A clock that went backwards is "just now", not a panic.
        assert_eq!(updated_label(1_000, 900), "updated just now");
        assert_eq!(updated_label(0, 1_000), "");
        assert_eq!(checks_label(&report(0, 0, 0, vec![])), "");
        assert_eq!(
            checks_label(&report(
                0,
                0,
                0,
                vec![check("a", "pass"), check("b", "fail")]
            )),
            "2 checks"
        );
    }

    #[test]
    fn a_new_run_clears_the_report_and_only_the_newest_answer_lands() {
        let mut audit = Audit::default();
        assert!(matches!(audit.state, AuditState::Idle));
        let first = audit.begin();
        assert!(audit.is_loading());
        assert!(audit.finish(first, Ok(report(90, 9, 10, vec![]))));
        assert_eq!(audit.report().map(|r| r.score), Some(90));

        // Refresh: the old report is gone while the new run is out.
        let second = audit.begin();
        assert!(audit.report().is_none());
        let third = audit.begin();
        assert!(!audit.finish(second, Err("late".into())), "superseded");
        assert!(audit.is_loading());
        assert!(audit.finish(third, Err("boom".into())));
        assert!(matches!(&audit.state, AuditState::Failed(e) if e == "boom"));
    }

    #[test]
    fn remediation_is_carried_through_untouched() {
        // The view shows and sends `remediation.command` exactly as the core
        // built it (from its own literals); nothing here derives one.
        let mut c = check("SSH root login", "fail");
        c.remediation = Some(Remediation {
            summary: "s".into(),
            command: Some("sudo sshd -T | grep -i permitrootlogin".into()),
        });
        let r = report(0, 0, 1, vec![c]);
        let mut audit = Audit::default();
        let run = audit.begin();
        audit.finish(run, Ok(r));
        let cmd = audit.report().unwrap().checks[0]
            .remediation
            .as_ref()
            .and_then(|r| r.command.as_deref());
        assert_eq!(cmd, Some("sudo sshd -T | grep -i permitrootlogin"));
    }
}
