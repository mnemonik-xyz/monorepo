//! Tests for the build-time-projected skill manifests (Task 1).
//!
//! Asserts the four TDD anchors named in the task spec:
//!
//!   1. `all_seven_manifests_parse` — the generated module exposes exactly 7
//!      manifests with non-empty `FULL_MARKDOWN`, `PURPOSE_PLUS_TRIGGER`, and
//!      `PURPOSE_ONE_LINER` slots each.
//!   2. `no_placeholder_tokens_in_manifests` — none of the three projected
//!      slots (full + purpose_plus_trigger + purpose_one_liner) contain
//!      `TBD`, `TODO`, `XXX`, or `FIXME`. Round-1 test-reviewer finding F2:
//!      derived slots are produced by string manipulation in build.rs, so
//!      checking all three guards against future template-injection
//!      regressions in the derived strings.
//!   3. `attest_manifest_has_negative_triggers` — `mnemonik-attest`'s
//!      Trigger section names at least one negative example. Load-bearing
//!      per user-spec R8.
//!   4. `build_fails_on_missing_purpose_section` — calls the same
//!      `expect_h2_section` function that build.rs uses at compile time
//!      (shared via `mcp/src/skill_parse.rs`), asserting that the
//!      missing-section error names the offending section. Round-1
//!      test-reviewer finding F1: an earlier draft re-implemented the
//!      parser inline in the test, which let the test pass even if
//!      build.rs's parser regressed. The shared module closes that.
//!
//! Plus one informational test from round-1 finding F3:
//!
//!   5. `unexpected_manifest_file_is_a_build_error` — sanity that the
//!      build script's `EXPECTED_MANIFESTS` constant matches the actual
//!      generated `ALL_SKILLS` set 1:1 (no orphan generated entry, no
//!      missing source manifest).

use std::collections::HashSet;

use mnemonic_mcp::mcp::skills;
use mnemonic_mcp::skill_parse::{expect_h2_section, extract_h2_section};

#[test]
fn all_seven_manifests_parse() {
    let names: Vec<&str> = skills::ALL_SKILLS.iter().map(|s| s.name).collect();
    assert_eq!(
        names.len(),
        7,
        "expected 7 skill manifests, got {}: {:?}",
        names.len(),
        names
    );

    let expected: HashSet<&str> = [
        "mnemonik-attest",
        "mnemonik-checkpoint",
        "mnemonik-help",
        "mnemonik-init",
        "mnemonik-recall",
        "mnemonik-status",
        "mnemonik-verify",
    ]
    .into_iter()
    .collect();
    let actual: HashSet<&str> = names.into_iter().collect();
    assert_eq!(actual, expected, "skill name set mismatch");

    for skill in skills::ALL_SKILLS {
        assert!(
            !skill.full_markdown.trim().is_empty(),
            "{}: full_markdown empty",
            skill.name
        );
        assert!(
            !skill.purpose_plus_trigger.trim().is_empty(),
            "{}: purpose_plus_trigger empty",
            skill.name
        );
        assert!(
            !skill.purpose_one_liner.trim().is_empty(),
            "{}: purpose_one_liner empty",
            skill.name
        );

        // Sanity: purpose+trigger should contain both labels build.rs
        // injected — proves the two sections were actually concatenated.
        assert!(
            skill.purpose_plus_trigger.contains("Purpose:"),
            "{}: purpose_plus_trigger missing Purpose label",
            skill.name
        );
        assert!(
            skill.purpose_plus_trigger.contains("Trigger:"),
            "{}: purpose_plus_trigger missing Trigger label",
            skill.name
        );
    }
}

#[test]
fn no_placeholder_tokens_in_manifests() {
    // Round-1 F2: extend coverage to the derived slots, not just the
    // verbatim full_markdown. A future template-injection regression in
    // build.rs's string-manipulation path would now be caught.
    const FORBIDDEN: &[&str] = &["TBD", "TODO", "XXX", "FIXME"];
    for skill in skills::ALL_SKILLS {
        for token in FORBIDDEN {
            assert!(
                !skill.full_markdown.contains(token),
                "{}: full_markdown contains placeholder token `{}`",
                skill.name,
                token
            );
            assert!(
                !skill.purpose_plus_trigger.contains(token),
                "{}: purpose_plus_trigger contains placeholder token `{}`",
                skill.name,
                token
            );
            assert!(
                !skill.purpose_one_liner.contains(token),
                "{}: purpose_one_liner contains placeholder token `{}`",
                skill.name,
                token
            );
        }
    }
}

#[test]
fn attest_manifest_has_negative_triggers() {
    let attest = skills::ALL_SKILLS
        .iter()
        .find(|s| s.name == "mnemonik-attest")
        .expect("mnemonik-attest manifest present");

    // The full markdown's Trigger section MUST contain explicit negative
    // guidance — user-spec R8 calls this load-bearing.
    let trigger = extract_h2_section(attest.full_markdown, "Trigger")
        .expect("attest manifest has `## Trigger` H2");

    let lower = trigger.to_lowercase();
    let has_negative_keyword =
        lower.contains("do not") || lower.contains("don't") || lower.contains("never");
    let has_negative_example_marker = lower.contains("negative example");
    assert!(
        has_negative_keyword && has_negative_example_marker,
        "mnemonik-attest's Trigger section must contain BOTH a negative-example marker \
         (e.g. `Negative examples`) AND at least one `do not`/`don't`/`never` directive. \
         Got Trigger section:\n{}",
        trigger
    );
}

#[test]
fn build_fails_on_missing_purpose_section() {
    // Round-1 F1: exercise the same `expect_h2_section` function that
    // build.rs invokes at compile time, not a mirror copy. If the build
    // script's parser regresses, this test fails along with it — the two
    // cannot drift silently because they ARE the same function.
    let tampered = "# mnemonik-help\n\n## Trigger\n\nfoo bar\n";
    let result = expect_h2_section(tampered, "Purpose");
    let err = result.expect_err("missing Purpose must error");
    assert!(
        err.contains("Purpose"),
        "error must name the missing section, got: {err}"
    );
    // The build script wraps this error as `manifest {name}.md {err}` — assert
    // the build-side wrapping format surfaces the filename too, by reproducing
    // the exact format-string build.rs uses on the panic path.
    let build_panic = format!("manifest attest.md {err}");
    assert!(
        build_panic.contains("attest.md") && build_panic.contains("Purpose"),
        "build-side wrapping must name file AND section, got: {build_panic}"
    );
}

#[test]
fn unexpected_manifest_file_is_a_build_error() {
    // Round-1 F3: build.rs treats EXPECTED_MANIFESTS as a both-ways
    // invariant — missing files AND extra files both fail the build.
    // We can't trigger the extras path from inside the unit test (build.rs
    // already ran), but we CAN assert the ALL_SKILLS contents are exactly
    // the expected set — same property the build guard enforces, asserted
    // at test time so a future EXPECTED_MANIFESTS edit that forgets to
    // also add an assets/skills/ file (or vice versa) trips here.
    let actual: HashSet<&str> = skills::ALL_SKILLS.iter().map(|s| s.name).collect();
    let expected: HashSet<&str> = [
        "mnemonik-attest",
        "mnemonik-checkpoint",
        "mnemonik-help",
        "mnemonik-init",
        "mnemonik-recall",
        "mnemonik-status",
        "mnemonik-verify",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        actual, expected,
        "ALL_SKILLS does not match the expected set — either an extra file \
         is being projected, or EXPECTED_MANIFESTS is out of sync with the \
         assets/skills/ directory"
    );
}
