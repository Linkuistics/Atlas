//! Per-run result files (YAML) and a static trend page (HTML).
//!
//! The trend page is deliberately dependency-free: a hand-rolled HTML
//! string assembled from every `*.yaml` under `evaluation/results/`.
//! No templating crate, no JS charting library — the goal is that
//! `trend.html` opens in any browser and reads clearly from the source.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::diff::MetricSummary;
use crate::invariants::InvariantReport;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultFile {
    /// ISO-8601 timestamp, date-only (YYYY-MM-DD) is acceptable.
    pub generated_at: String,
    /// Free-form target label — e.g., `tiny`, `dev-workspace`,
    /// `merged-monorepo`.
    pub target: String,
    /// Absent when the run had no golden to compare against (tiny
    /// fixtures, smoke tests).
    pub metrics: Option<MetricSummary>,
    pub invariants: InvariantReport,
    /// Optional free-form notes about the run (iteration number, flags
    /// passed, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Atomic-ish write: build the YAML in memory then `fs::write`. No
/// temp-file dance because `evaluation/results/` is not on the critical
/// path — a crashed write is easy to redo.
pub fn write_result_yaml(path: &Path, result: &ResultFile) -> Result<()> {
    let yaml = serde_yaml::to_string(result).context("serialise ResultFile to yaml")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create result parent directory {}", parent.display()))?;
    }
    fs::write(path, yaml).with_context(|| format!("write result yaml to {}", path.display()))?;
    Ok(())
}

/// Scan `results_dir` for `*.yaml` files, parse each as a `ResultFile`,
/// and render a minimal static HTML trend page at `output`. Skips files
/// that don't parse (to avoid a single malformed blob breaking the
/// whole page) but records their names in a footer.
pub fn render_trend_html(results_dir: &Path, output: &Path) -> Result<()> {
    let mut results: Vec<(PathBuf, ResultFile)> = Vec::new();
    let mut skipped: Vec<PathBuf> = Vec::new();

    if results_dir.exists() {
        for entry in fs::read_dir(results_dir)
            .with_context(|| format!("read {}", results_dir.display()))?
            .flatten()
        {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "yaml") {
                continue;
            }
            let content = fs::read_to_string(&path)
                .with_context(|| format!("read result yaml {}", path.display()))?;
            match serde_yaml::from_str::<ResultFile>(&content) {
                Ok(r) => results.push((path, r)),
                Err(_) => skipped.push(path),
            }
        }
    }

    results.sort_by(|a, b| a.1.generated_at.cmp(&b.1.generated_at));

    let html = render_html(&results, &skipped);
    fs::write(output, html).with_context(|| format!("write trend html to {}", output.display()))?;
    Ok(())
}

fn render_html(results: &[(PathBuf, ResultFile)], skipped: &[PathBuf]) -> String {
    let mut out = String::new();
    out.push_str(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Atlas evaluation trend</title>
<style>
body { font-family: -apple-system, system-ui, sans-serif; margin: 2em; }
table { border-collapse: collapse; }
th, td { border: 1px solid #ccc; padding: 0.4em 0.7em; text-align: left; }
th { background: #f5f5f5; }
tr.fail td { background: #ffefef; }
tr.no-golden td.metric { color: #888; font-style: italic; }
.num { font-variant-numeric: tabular-nums; text-align: right; }
.footer { margin-top: 2em; font-size: 0.85em; color: #666; }
</style>
</head>
<body>
<h1>Atlas evaluation trend</h1>
<table>
<thead>
<tr>
<th>Date</th>
<th>Target</th>
<th class="num">Component coverage</th>
<th class="num">Spurious rate</th>
<th class="num">Kind accuracy</th>
<th class="num">Edge precision</th>
<th class="num">Edge recall</th>
<th class="num">ID stability</th>
<th>Invariants</th>
</tr>
</thead>
<tbody>
"#,
    );

    for (_, result) in results {
        let failing = !result.invariants.all_passed();
        let no_golden = result.metrics.is_none();
        let tr_class = match (failing, no_golden) {
            (true, _) => " class=\"fail\"",
            (false, true) => " class=\"no-golden\"",
            _ => "",
        };
        out.push_str(&format!("<tr{tr_class}>"));
        out.push_str(&format!("<td>{}</td>", escape(&result.generated_at)));
        out.push_str(&format!("<td>{}</td>", escape(&result.target)));
        match &result.metrics {
            Some(m) => {
                out.push_str(&format!(
                    "<td class=\"num\">{:.3}</td>",
                    m.component_coverage
                ));
                out.push_str(&format!("<td class=\"num\">{:.3}</td>", m.spurious_rate));
                out.push_str(&format!("<td class=\"num\">{:.3}</td>", m.kind_accuracy));
                out.push_str(&format!("<td class=\"num\">{:.3}</td>", m.edge_precision));
                out.push_str(&format!("<td class=\"num\">{:.3}</td>", m.edge_recall));
                out.push_str(&format!(
                    "<td class=\"num\">{}</td>",
                    m.identifier_stability
                        .map(|v| format!("{v:.3}"))
                        .unwrap_or_else(|| "—".into())
                ));
            }
            None => {
                for _ in 0..6 {
                    out.push_str("<td class=\"metric\">—</td>");
                }
            }
        }
        let inv_summary = summarise_invariants(&result.invariants);
        out.push_str(&format!("<td>{}</td>", escape(&inv_summary)));
        out.push_str("</tr>\n");
    }

    out.push_str("</tbody></table>\n");

    if !skipped.is_empty() {
        out.push_str("<div class=\"footer\">Skipped unparsable YAMLs: ");
        let names: Vec<String> = skipped
            .iter()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        out.push_str(&escape(&names.join(", ")));
        out.push_str("</div>\n");
    }

    out.push_str("</body></html>\n");
    out
}

fn summarise_invariants(report: &InvariantReport) -> String {
    let total = report.outcomes.len();
    let failed: Vec<&String> = report.failures().map(|(k, _)| k).collect();
    if failed.is_empty() {
        format!("{total}/{total} passed")
    } else {
        let names: Vec<String> = failed.iter().map(|s| (*s).clone()).collect();
        format!("{} failed: {}", failed.len(), names.join(", "))
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::TempDir;

    use super::*;
    use crate::invariants::InvariantOutcome;

    fn sample(generated_at: &str, target: &str, pass: bool) -> ResultFile {
        let mut outcomes: BTreeMap<String, InvariantOutcome> = BTreeMap::new();
        outcomes.insert("path_coverage".into(), InvariantOutcome::Pass);
        outcomes.insert(
            "edge_participant_existence".into(),
            if pass {
                InvariantOutcome::Pass
            } else {
                InvariantOutcome::Fail {
                    message: "ghost".into(),
                }
            },
        );
        ResultFile {
            generated_at: generated_at.into(),
            target: target.into(),
            metrics: Some(MetricSummary {
                component_coverage: 0.91,
                spurious_rate: 0.08,
                kind_accuracy: 0.86,
                edge_precision: 0.80,
                edge_recall: 0.77,
                identifier_stability: Some(0.99),
            }),
            invariants: InvariantReport { outcomes },
            notes: None,
        }
    }

    #[test]
    fn write_and_parse_result_file_round_trips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("2026-04-24-tiny.yaml");
        let result = sample("2026-04-24", "tiny", true);
        write_result_yaml(&path, &result).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: ResultFile = serde_yaml::from_str(&content).unwrap();
        assert_eq!(parsed.target, "tiny");
        assert!(parsed.invariants.all_passed());
    }

    #[test]
    fn trend_html_renders_single_result() {
        let tmp = TempDir::new().unwrap();
        let results_dir = tmp.path().join("results");
        std::fs::create_dir(&results_dir).unwrap();
        write_result_yaml(
            &results_dir.join("2026-04-24-tiny.yaml"),
            &sample("2026-04-24", "tiny", true),
        )
        .unwrap();

        let out = tmp.path().join("trend.html");
        render_trend_html(&results_dir, &out).unwrap();
        let html = std::fs::read_to_string(&out).unwrap();
        assert!(html.contains("Atlas evaluation trend"));
        assert!(html.contains("tiny"));
        assert!(
            html.contains("0.910"),
            "expected coverage metric in html: {html}"
        );
    }

    #[test]
    fn trend_html_marks_failing_invariants() {
        let tmp = TempDir::new().unwrap();
        let results_dir = tmp.path().join("results");
        std::fs::create_dir(&results_dir).unwrap();
        write_result_yaml(
            &results_dir.join("2026-04-24-tiny.yaml"),
            &sample("2026-04-24", "tiny", false),
        )
        .unwrap();
        let out = tmp.path().join("trend.html");
        render_trend_html(&results_dir, &out).unwrap();
        let html = std::fs::read_to_string(&out).unwrap();
        assert!(
            html.contains("class=\"fail\""),
            "failing rows should be flagged: {html}"
        );
        assert!(html.contains("edge_participant_existence"));
    }

    #[test]
    fn trend_html_handles_empty_results_dir() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("trend.html");
        render_trend_html(&tmp.path().join("does-not-exist"), &out).unwrap();
        let html = std::fs::read_to_string(&out).unwrap();
        assert!(html.contains("Atlas evaluation trend"));
    }

    #[test]
    fn trend_html_sorts_by_generated_at() {
        let tmp = TempDir::new().unwrap();
        let results_dir = tmp.path().join("results");
        std::fs::create_dir(&results_dir).unwrap();
        write_result_yaml(
            &results_dir.join("later.yaml"),
            &sample("2026-05-01", "tiny", true),
        )
        .unwrap();
        write_result_yaml(
            &results_dir.join("earlier.yaml"),
            &sample("2026-04-01", "tiny", true),
        )
        .unwrap();
        let out = tmp.path().join("trend.html");
        render_trend_html(&results_dir, &out).unwrap();
        let html = std::fs::read_to_string(&out).unwrap();
        let earlier_pos = html.find("2026-04-01").unwrap();
        let later_pos = html.find("2026-05-01").unwrap();
        assert!(earlier_pos < later_pos, "rows should be chronological");
    }
}
