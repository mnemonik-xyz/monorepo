// Shared parser for the skill manifests under `mcp/assets/skills/*.md`.
//
// This file is the single source of truth for the H2-section extraction
// used to project each manifest into compile-time string constants. It is
// consumed two ways:
//
//   1. `mcp/build.rs` includes it via `include!(...)` at build time to do
//      the projection (it cannot `use` it — the build script runs before
//      the crate compiles).
//   2. `mcp/tests/skill_manifests.rs` imports it via the crate (`use
//      mnemonic_mcp::skill_parse::...`) so the build guard's "missing
//      `## Purpose` fails" behaviour is tested against the exact same
//      code that runs at build time. If this file changes shape, both
//      sides update together — no silent drift between test and build.
//
// Inner doc-comments (`//!`) are intentionally NOT used here — when this
// file is `include!()`-d into build.rs, the inner-doc syntax errors out
// because include! splices at item position, not module position.

/// Extract the body of an H2 section with the given title from a markdown
/// document. Returns the text between the `## <title>` line (exclusive) and
/// the next `## ` or `# ` line (exclusive), or None if the section header
/// is not present.
///
/// Header matching is exact on the line (after trimming) — `## Purpose`
/// matches but `## Purpose-driven` does not, so a stray heading containing
/// the title as a prefix never satisfies the parser.
pub fn extract_h2_section(body: &str, title: &str) -> Option<String> {
    let header = format!("## {title}");
    let all_lines: Vec<&str> = body.lines().collect();
    let start = all_lines.iter().position(|l| l.trim() == header)? + 1;
    let end = all_lines
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(i, l)| {
            if l.starts_with("## ") || l.starts_with("# ") {
                Some(i)
            } else {
                None
            }
        })
        .unwrap_or(all_lines.len());
    Some(all_lines[start..end].join("\n"))
}

/// Like [`extract_h2_section`] but returns a `Result` whose `Err` names the
/// missing section. Build.rs uses this through `unwrap_or_else(|e| panic!)`
/// so the build failure message contains the section name; the test surface
/// uses it to assert the same error surface.
pub fn expect_h2_section(body: &str, title: &str) -> Result<String, String> {
    extract_h2_section(body, title)
        .ok_or_else(|| format!("manifest missing required `## {title}` H2 section"))
}

/// Format the exact panic message that build.rs emits when a manifest is
/// missing a required H2 section. Single source of truth so the integration
/// test asserts on the same string the build script produces — adding a new
/// piece of context to the error (e.g., a repair hint) requires updating
/// this function, and the test then immediately reflects the new shape.
///
/// Security-auditor round 1 SA-T1-02: the missing-section test asserts on
/// the output of this function, threading both the manifest name AND the
/// section title through, so a regression that drops either from the build
/// panic surfaces in the test.
pub fn format_missing_section_panic(manifest_name: &str, parse_err: &str) -> String {
    format!("manifest {manifest_name}.md {parse_err}")
}
