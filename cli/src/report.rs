//! Standalone interactive HTML5 storage audit report generator.
//!
//! Produces a completely self-contained single-file HTML report (zero external CDN
//! dependencies) featuring:
//! - KPI summary cards
//! - Interactive SVG ecosystem donut chart with clickable slice filtering
//! - Interactive SVG staleness timeline bar chart
//! - Client-side searchable, filterable, sortable artifact inspection table
//! - Strict HTML entity and `<script>` JSON escaping to prevent XSS injection

use anyhow::Result;
use nodewipe_core::explain::analyze_artifact_rule;
use nodewipe_core::history::{format_date, now_unix};
use nodewipe_core::{
    ArtifactKind, Entry, RiskLevel, StaleProject, is_path_pinned_with_list, load_pinned_projects,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::SystemTime;

/// Escapes characters with special meaning in HTML text and attribute values.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Escapes JSON text so it cannot prematurely close a `<script>` tag or trigger
/// HTML/JS parser ambiguities when embedded directly inside HTML5.
pub fn escape_json_for_script(json: &str) -> String {
    json.replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

#[derive(Debug, Clone, Serialize)]
pub struct HtmlReportEntry {
    pub path: String,
    pub kind_slug: String,
    pub kind_label: String,
    pub color: String,
    pub size_bytes: u64,
    pub size_formatted: String,
    pub age_days: Option<u64>,
    pub age_bucket: String,
    pub risk: String,
    pub git_branch: Option<String>,
    pub git_dirty: bool,
    pub git_merged: bool,
    pub workspace_root: Option<String>,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EcosystemSummary {
    pub label: String,
    pub color: String,
    pub count: usize,
    pub total_bytes: u64,
    pub size_formatted: String,
    pub percentage: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgeBucketSummary {
    pub key: String,
    pub label: String,
    pub count: usize,
    pub total_bytes: u64,
    pub size_formatted: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HtmlStaleProject {
    pub root: String,
    pub stale_days: u64,
    pub reclaimable_bytes: u64,
    pub reclaimable_formatted: String,
    pub git_branch: Option<String>,
    pub is_dirty: bool,
    pub is_merged: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HtmlReportPayload {
    pub root: String,
    pub generated_date: String,
    pub total_bytes: u64,
    pub total_formatted: String,
    pub workspace_count: usize,
    pub oldest_days: Option<u64>,
    pub entries: Vec<HtmlReportEntry>,
    pub ecosystems: Vec<EcosystemSummary>,
    pub age_buckets: Vec<AgeBucketSummary>,
    pub stale_projects: Vec<HtmlStaleProject>,
}

fn ecosystem_color(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::NodeModules
        | ArtifactKind::NextCache
        | ArtifactKind::TurboCache
        | ArtifactKind::GenericDist
        | ArtifactKind::AstroCache
        | ArtifactKind::SvelteKitCache
        | ArtifactKind::NuxtCache
        | ArtifactKind::DocusaurusCache
        | ArtifactKind::ViteCache
        | ArtifactKind::PnpmIgnored
        | ArtifactKind::YarnCache => "#22c55e",
        ArtifactKind::RustTarget => "#f97316",
        ArtifactKind::PythonVenv
        | ArtifactKind::PythonPycache
        | ArtifactKind::PythonPytestCache
        | ArtifactKind::PythonMypyCache
        | ArtifactKind::PythonRuffCache => "#eab308",
        ArtifactKind::JavaMavenTarget
        | ArtifactKind::JavaGradleBuild
        | ArtifactKind::GradleCache
        | ArtifactKind::AndroidBuildCache => "#ec4899",
        ArtifactKind::GoModCache | ArtifactKind::GoBuildCache => "#06b6d4",
        ArtifactKind::DotnetBin | ArtifactKind::DotnetObj => "#a855f7",
        ArtifactKind::PhpVendor => "#6366f1",
        ArtifactKind::HuggingFaceCache
        | ArtifactKind::OllamaModels
        | ArtifactKind::TorchCache
        | ArtifactKind::TritonCache => "#f59e0b",
        ArtifactKind::XcodeDerivedData | ArtifactKind::SwiftPmBuild | ArtifactKind::CocoaPods => {
            "#3b82f6"
        }
        ArtifactKind::CmakeBuild | ArtifactKind::ZigCache | ArtifactKind::MesonBuild => "#14b8a6",
        ArtifactKind::PlaywrightBrowsers
        | ArtifactKind::CypressCache
        | ArtifactKind::ElixirBuild
        | ArtifactKind::Custom => "#94a3b8",
    }
}

fn format_bytes(bytes: u64) -> String {
    nodewipe_core::explain::format_size(bytes)
}

fn classify_age_bucket(age_days: Option<u64>) -> (&'static str, &'static str) {
    match age_days {
        Some(d) if d < 7 => ("<7d", "< 7 days"),
        Some(d) if d < 30 => ("7-30d", "7 - 30 days"),
        Some(d) if d < 90 => ("30-90d", "30 - 90 days"),
        Some(d) if d < 180 => ("90-180d", "90 - 180 days"),
        Some(_) => ("180d+", "180+ days"),
        None => ("unknown", "Unknown"),
    }
}

/// Builds structured report telemetry from scan entries and stale project analysis.
pub fn build_report_payload(
    root: &Path,
    entries: &[Entry],
    stale: &[StaleProject],
) -> HtmlReportPayload {
    let now = SystemTime::now();
    let pinned_projects = load_pinned_projects();
    let total_bytes: u64 = entries.iter().map(|e| e.size_bytes).sum();
    let generated_date = format_date(now_unix());

    let mut workspaces: HashSet<String> = HashSet::new();
    let mut oldest_days: Option<u64> = None;

    let mut eco_map: BTreeMap<String, (String, usize, u64)> = BTreeMap::new();
    let bucket_order = [
        ("<7d", "< 7 days"),
        ("7-30d", "7 - 30 days"),
        ("30-90d", "30 - 90 days"),
        ("90-180d", "90 - 180 days"),
        ("180d+", "180+ days"),
        ("unknown", "Unknown"),
    ];
    let mut bucket_map: BTreeMap<&'static str, (usize, u64)> = BTreeMap::new();
    for (k, _) in &bucket_order {
        bucket_map.insert(*k, (0, 0));
    }

    let mut report_entries = Vec::with_capacity(entries.len());

    for e in entries {
        let age_days = e
            .last_modified
            .and_then(|lm| now.duration_since(lm).ok())
            .map(|d| d.as_secs() / 86400);

        if let Some(d) = age_days {
            oldest_days = Some(oldest_days.map_or(d, |cur| cur.max(d)));
        }

        let (bucket_key, _) = classify_age_bucket(age_days);
        let b_entry = bucket_map.entry(bucket_key).or_insert((0, 0));
        b_entry.0 += 1;
        b_entry.1 = b_entry.1.saturating_add(e.size_bytes);

        let kind_label = e.display_label().to_string();
        let color = ecosystem_color(e.kind).to_string();
        let eco_entry = eco_map
            .entry(kind_label.clone())
            .or_insert_with(|| (color.clone(), 0, 0));
        eco_entry.1 += 1;
        eco_entry.2 = eco_entry.2.saturating_add(e.size_bytes);

        if let Some(ws) = &e.workspace_root {
            workspaces.insert(ws.to_string_lossy().to_string());
        }

        let risk_lvl = if let Some(rule) = &e.custom_rule {
            rule.risk_level()
        } else {
            analyze_artifact_rule(e.kind, &e.path).1
        };
        let risk = match risk_lvl {
            RiskLevel::Low => "low",
            RiskLevel::Moderate => "moderate",
            RiskLevel::Caution => "caution",
        }
        .to_string();

        let (git_branch, git_dirty, git_merged) = match &e.git_status {
            Some(g) => (g.branch.clone(), g.is_dirty, g.is_merged),
            None => (None, false, false),
        };

        let pinned = is_path_pinned_with_list(&e.path, &pinned_projects);

        report_entries.push(HtmlReportEntry {
            path: e.path.to_string_lossy().to_string(),
            kind_slug: e.kind.slug().to_string(),
            kind_label,
            color,
            size_bytes: e.size_bytes,
            size_formatted: format_bytes(e.size_bytes),
            age_days,
            age_bucket: bucket_key.to_string(),
            risk,
            git_branch,
            git_dirty,
            git_merged,
            workspace_root: e
                .workspace_root
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            pinned,
        });
    }

    let mut ecosystems: Vec<EcosystemSummary> = eco_map
        .into_iter()
        .map(|(label, (color, count, bytes))| {
            let percentage = if total_bytes > 0 {
                (bytes as f64 / total_bytes as f64) * 100.0
            } else {
                0.0
            };
            EcosystemSummary {
                label,
                color,
                count,
                total_bytes: bytes,
                size_formatted: format_bytes(bytes),
                percentage,
            }
        })
        .collect();
    ecosystems.sort_by_key(|a| std::cmp::Reverse(a.total_bytes));

    let age_buckets: Vec<AgeBucketSummary> = bucket_order
        .iter()
        .filter_map(|&(key, label)| {
            let &(count, bytes) = bucket_map.get(key)?;
            Some(AgeBucketSummary {
                key: key.to_string(),
                label: label.to_string(),
                count,
                total_bytes: bytes,
                size_formatted: format_bytes(bytes),
            })
        })
        .collect();

    let stale_projects: Vec<HtmlStaleProject> = stale
        .iter()
        .map(|s| HtmlStaleProject {
            root: s.root.to_string_lossy().to_string(),
            stale_days: s.stale_days,
            reclaimable_bytes: s.reclaimable_bytes,
            reclaimable_formatted: format_bytes(s.reclaimable_bytes),
            git_branch: s.git_branch.clone(),
            is_dirty: s.is_dirty,
            is_merged: s.is_merged,
        })
        .collect();

    HtmlReportPayload {
        root: root.to_string_lossy().to_string(),
        generated_date,
        total_bytes,
        total_formatted: format_bytes(total_bytes),
        workspace_count: workspaces.len(),
        oldest_days,
        entries: report_entries,
        ecosystems,
        age_buckets,
        stale_projects,
    }
}

/// Renders an inline SVG donut chart using stroke-dasharray segments around a circle.
fn render_donut_svg(ecosystems: &[EcosystemSummary], total_formatted: &str) -> String {
    if ecosystems.is_empty() {
        return String::from(
            r#"<svg viewBox="0 0 220 220" class="donut-svg" role="img" aria-label="No ecosystem data"><circle cx="110" cy="110" r="80" fill="none" stroke="var(--border)" stroke-width="26"></circle><text x="110" y="114" text-anchor="middle" class="donut-center-label">0 B</text></svg>"#,
        );
    }

    let radius = 78.0_f64;
    let circumference = 2.0 * std::f64::consts::PI * radius;
    let mut offset = 0.0_f64;
    let mut circles = String::new();

    for eco in ecosystems {
        let fraction = (eco.percentage / 100.0).clamp(0.0, 1.0);
        let dash = fraction * circumference;
        let gap = (circumference - dash).max(0.0);
        let escaped_label = escape_html(&eco.label);
        let escaped_size = escape_html(&eco.size_formatted);
        circles.push_str(&format!(
            r#"<circle class="donut-slice" data-ecosystem="{escaped_label}" cx="110" cy="110" r="{radius}" fill="none" stroke="{color}" stroke-width="28" stroke-dasharray="{dash:.2} {gap:.2}" stroke-dashoffset="-{offset:.2}" transform="rotate(-90 110 110)"><title>{escaped_label}: {escaped_size} ({pct:.1}%)</title></circle>"#,
            color = escape_html(&eco.color),
            pct = eco.percentage
        ));
        offset += dash;
    }

    let escaped_total = escape_html(total_formatted);
    format!(
        r#"<svg viewBox="0 0 220 220" class="donut-svg" role="img" aria-label="Storage distribution by ecosystem">{circles}<text x="110" y="106" text-anchor="middle" class="donut-center-value">{escaped_total}</text><text x="110" y="126" text-anchor="middle" class="donut-center-sub">reclaimable</text></svg>"#
    )
}

/// Renders an inline SVG bar chart of artifact storage distributed by age bucket.
fn render_age_bars_svg(buckets: &[AgeBucketSummary]) -> String {
    let max_bytes = buckets
        .iter()
        .map(|b| b.total_bytes)
        .max()
        .unwrap_or(1)
        .max(1);

    let width = 460.0_f64;
    let height = 210.0_f64;
    let chart_h = 145.0_f64;
    let top_pad = 22.0_f64;
    let slot_w = width / (buckets.len().max(1) as f64);
    let bar_w = (slot_w * 0.56).min(48.0);

    let mut bars = String::new();
    for (idx, b) in buckets.iter().enumerate() {
        let ratio = (b.total_bytes as f64) / (max_bytes as f64);
        let bar_h = if b.total_bytes > 0 {
            (ratio * chart_h).max(4.0)
        } else {
            2.0
        };
        let x = (idx as f64) * slot_w + (slot_w - bar_w) / 2.0;
        let y = top_pad + (chart_h - bar_h);
        let cx = x + bar_w / 2.0;
        let key_esc = escape_html(&b.key);
        let size_esc = escape_html(&b.size_formatted);
        let label_esc = escape_html(&b.label);
        let fill = match b.key.as_str() {
            "180d+" => "#ef4444",
            "90-180d" => "#f97316",
            "30-90d" => "#eab308",
            "7-30d" => "#3b82f6",
            "<7d" => "#22c55e",
            _ => "#64748b",
        };
        bars.push_str(&format!(
            r#"<g class="age-bar-group" data-age-bucket="{key_esc}"><title>{label_esc}: {size_esc} ({count} artifacts)</title><rect x="{x:.1}" y="{y:.1}" width="{bar_w:.1}" height="{bar_h:.1}" rx="4" fill="{fill}" class="age-bar" /><text x="{cx:.1}" y="{val_y:.1}" text-anchor="middle" class="bar-val">{size_esc}</text><text x="{cx:.1}" y="{lbl_y:.1}" text-anchor="middle" class="bar-lbl">{key_esc}</text><text x="{cx:.1}" y="{cnt_y:.1}" text-anchor="middle" class="bar-cnt">{count}x</text></g>"#,
            count = b.count,
            val_y = (y - 6.0).max(12.0),
            lbl_y = top_pad + chart_h + 18.0,
            cnt_y = top_pad + chart_h + 33.0,
        ));
    }

    format!(
        r#"<svg viewBox="0 0 {width:.0} {height:.0}" class="age-svg" role="img" aria-label="Artifact staleness distribution">{bars}</svg>"#
    )
}

/// Generates a standalone, self-contained HTML5 storage audit report.
pub fn generate_html_report(
    root: &Path,
    entries: &[Entry],
    stale: &[StaleProject],
) -> Result<String> {
    let payload = build_report_payload(root, entries, stale);
    let json_raw = serde_json::to_string(&payload)?;
    let json_safe = escape_json_for_script(&json_raw);

    let root_esc = escape_html(&payload.root);
    let date_esc = escape_html(&payload.generated_date);
    let total_esc = escape_html(&payload.total_formatted);
    let oldest_str = payload
        .oldest_days
        .map(|d| format!("{d}d"))
        .unwrap_or_else(|| "N/A".to_string());

    let donut_svg = render_donut_svg(&payload.ecosystems, &payload.total_formatted);
    let age_svg = render_age_bars_svg(&payload.age_buckets);

    let mut legend_html = String::new();
    for eco in &payload.ecosystems {
        let lbl = escape_html(&eco.label);
        let col = escape_html(&eco.color);
        let sz = escape_html(&eco.size_formatted);
        legend_html.push_str(&format!(
            r#"<button type="button" class="legend-item" data-ecosystem="{lbl}"><span class="legend-swatch" style="background:{col}"></span><span class="legend-name">{lbl}</span><span class="legend-meta">{sz} ({pct:.1}%, {cnt})</span></button>"#,
            pct = eco.percentage,
            cnt = eco.count
        ));
    }

    let mut eco_options = String::from(r#"<option value="">All Ecosystems</option>"#);
    for eco in &payload.ecosystems {
        let lbl = escape_html(&eco.label);
        eco_options.push_str(&format!(
            r#"<option value="{lbl}">{lbl} ({cnt})</option>"#,
            cnt = eco.count
        ));
    }

    let mut age_options = String::from(r#"<option value="">All Ages</option>"#);
    for b in &payload.age_buckets {
        let k = escape_html(&b.key);
        let lbl = escape_html(&b.label);
        age_options.push_str(&format!(
            r#"<option value="{k}">{lbl} ({cnt})</option>"#,
            cnt = b.count
        ));
    }

    let stale_section = if payload.stale_projects.is_empty() {
        String::new()
    } else {
        let mut rows = String::new();
        for s in &payload.stale_projects {
            let s_root = escape_html(&s.root);
            let s_size = escape_html(&s.reclaimable_formatted);
            let git_badge = match &s.git_branch {
                Some(br) => {
                    let dirty = if s.is_dirty { "*" } else { "" };
                    let merged = if s.is_merged { " merged" } else { "" };
                    format!(
                        r#"<span class="badge badge-git">{}{dirty}{merged}</span>"#,
                        escape_html(br)
                    )
                }
                None => String::new(),
            };
            rows.push_str(&format!(
                r#"<tr><td class="mono">{s_root} {git_badge}</td><td>{days}d</td><td class="num">{s_size}</td></tr>"#,
                days = s.stale_days
            ));
        }
        format!(
            r#"<section class="panel"><h2>Likely-Abandoned Projects ({count})</h2><div class="table-wrap"><table><thead><tr><th>Project Root</th><th>Source Untouched</th><th class="num">Reclaimable</th></tr></thead><tbody>{rows}</tbody></table></div></section>"#,
            count = payload.stale_projects.len()
        )
    };

    // Pre-render SSR fallback rows so non-JS viewers and static snapshots contain full content
    let mut ssr_rows = String::new();
    for item in &payload.entries {
        let p = escape_html(&item.path);
        let k = escape_html(&item.kind_label);
        let c = escape_html(&item.color);
        let sz = escape_html(&item.size_formatted);
        let age = item
            .age_days
            .map(|d| format!("{d}d"))
            .unwrap_or_else(|| "?".to_string());
        let risk = escape_html(&item.risk);
        ssr_rows.push_str(&format!(
            r#"<tr><td><span class="badge-eco" style="--eco:{c}">{k}</span></td><td class="mono">{p}</td><td class="num">{sz}</td><td class="num">{age}</td><td><span class="badge badge-risk-{risk}">{risk}</span></td><td class="actions-cell">-</td></tr>"#
        ));
    }

    let version = env!("CARGO_PKG_VERSION");

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en" data-theme="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>nodewipe Storage Audit Report - {root_esc}</title>
<style>
:root {{
  --bg: #0f172a;
  --surface: #1e293b;
  --surface-alt: #26334a;
  --text: #f1f5f9;
  --muted: #94a3b8;
  --border: #334155;
  --accent: #38bdf8;
  --green: #22c55e;
  --yellow: #eab308;
  --red: #ef4444;
  --mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  --sans: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
}}
html[data-theme="light"] {{
  --bg: #f8fafc;
  --surface: #ffffff;
  --surface-alt: #f1f5f9;
  --text: #0f172a;
  --muted: #475569;
  --border: #cbd5e1;
  --accent: #0284c7;
}}
* {{ box-sizing: border-box; }}
body {{
  margin: 0;
  font-family: var(--sans);
  background: var(--bg);
  color: var(--text);
  line-height: 1.5;
}}
.container {{
  max-width: 1320px;
  margin: 0 auto;
  padding: 24px 20px 48px;
}}
header.top-bar {{
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
  padding-bottom: 18px;
  border-bottom: 1px solid var(--border);
  margin-bottom: 24px;
}}
.brand h1 {{
  margin: 0;
  font-size: 1.5rem;
  letter-spacing: -0.02em;
}}
.brand .subtitle {{
  margin: 4px 0 0;
  color: var(--muted);
  font-size: 0.9rem;
  font-family: var(--mono);
  word-break: break-all;
}}
.header-actions {{
  display: flex;
  align-items: center;
  gap: 10px;
}}
.meta-pill {{
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 6px 12px;
  font-size: 0.8rem;
  color: var(--muted);
}}
button.btn {{
  cursor: pointer;
  background: var(--surface);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 6px 12px;
  font-size: 0.85rem;
  transition: background 0.15s, border-color 0.15s;
}}
button.btn:hover {{
  background: var(--surface-alt);
  border-color: var(--accent);
}}
.kpi-grid {{
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(210px, 1fr));
  gap: 14px;
  margin-bottom: 22px;
}}
.kpi-card {{
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 16px 18px;
}}
.kpi-label {{
  font-size: 0.78rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--muted);
}}
.kpi-value {{
  margin-top: 6px;
  font-size: 1.65rem;
  font-weight: 700;
}}
.charts-grid {{
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(360px, 1fr));
  gap: 18px;
  margin-bottom: 22px;
}}
.panel {{
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 18px;
  margin-bottom: 20px;
}}
.panel h2 {{
  margin: 0 0 14px;
  font-size: 1.05rem;
  font-weight: 600;
}}
.donut-wrap {{
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 18px;
}}
.donut-svg {{
  width: 210px;
  height: 210px;
  flex-shrink: 0;
}}
.donut-slice {{
  cursor: pointer;
  transition: opacity 0.15s, stroke-width 0.15s;
}}
.donut-slice:hover, .donut-slice.active {{
  stroke-width: 33;
}}
.donut-center-value {{
  fill: var(--text);
  font-size: 20px;
  font-weight: 700;
}}
.donut-center-sub, .donut-center-label {{
  fill: var(--muted);
  font-size: 12px;
}}
.legend {{
  flex: 1;
  min-width: 180px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: 215px;
  overflow-y: auto;
}}
.legend-item {{
  display: flex;
  align-items: center;
  gap: 8px;
  background: transparent;
  border: 1px solid transparent;
  color: var(--text);
  padding: 4px 8px;
  border-radius: 6px;
  font-size: 0.84rem;
  cursor: pointer;
  text-align: left;
}}
.legend-item:hover, .legend-item.active {{
  background: var(--surface-alt);
  border-color: var(--accent);
}}
.legend-swatch {{
  width: 10px;
  height: 10px;
  border-radius: 3px;
  flex-shrink: 0;
}}
.legend-name {{
  flex: 1;
  font-weight: 500;
}}
.legend-meta {{
  color: var(--muted);
  font-family: var(--mono);
  font-size: 0.78rem;
}}
.age-svg {{
  width: 100%;
  height: auto;
  max-height: 225px;
}}
.age-bar-group {{
  cursor: pointer;
}}
.age-bar {{
  transition: opacity 0.15s;
}}
.age-bar-group:hover .age-bar, .age-bar-group.active .age-bar {{
  opacity: 0.8;
  stroke: var(--text);
  stroke-width: 1.5;
}}
.bar-val {{
  fill: var(--text);
  font-size: 10px;
  font-family: var(--mono);
}}
.bar-lbl {{
  fill: var(--text);
  font-size: 11px;
  font-weight: 600;
}}
.bar-cnt {{
  fill: var(--muted);
  font-size: 10px;
}}
.controls {{
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  margin-bottom: 14px;
  align-items: center;
}}
.controls input, .controls select {{
  background: var(--bg);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 7px 11px;
  font-size: 0.88rem;
}}
.controls input {{
  flex: 1;
  min-width: 220px;
}}
.filter-status {{
  color: var(--muted);
  font-size: 0.85rem;
  margin-left: auto;
}}
.table-wrap {{
  overflow-x: auto;
}}
table {{
  width: 100%;
  border-collapse: collapse;
  font-size: 0.88rem;
}}
th, td {{
  padding: 10px 12px;
  border-bottom: 1px solid var(--border);
  text-align: left;
  vertical-align: middle;
}}
th {{
  color: var(--muted);
  font-weight: 600;
  font-size: 0.8rem;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  user-select: none;
}}
th.sortable {{
  cursor: pointer;
}}
th.sortable:hover {{
  color: var(--accent);
}}
tbody tr:hover {{
  background: var(--surface-alt);
}}
.mono {{
  font-family: var(--mono);
  word-break: break-all;
}}
.num {{
  text-align: right;
  font-family: var(--mono);
  white-space: nowrap;
}}
.badge-eco {{
  display: inline-block;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 0.78rem;
  font-weight: 600;
  border: 1px solid var(--eco);
  color: var(--eco);
  white-space: nowrap;
}}
.badge {{
  display: inline-block;
  padding: 2px 7px;
  border-radius: 4px;
  font-size: 0.74rem;
  font-weight: 600;
  margin-left: 4px;
}}
.badge-git {{
  background: rgba(56, 189, 248, 0.15);
  color: var(--accent);
}}
.badge-pin {{
  background: rgba(234, 179, 8, 0.15);
  color: var(--yellow);
}}
.badge-risk-low {{
  background: rgba(34, 197, 94, 0.15);
  color: var(--green);
}}
.badge-risk-moderate {{
  background: rgba(234, 179, 8, 0.15);
  color: var(--yellow);
}}
.badge-risk-caution {{
  background: rgba(239, 68, 68, 0.15);
  color: var(--red);
}}
.actions-cell {{
  white-space: nowrap;
}}
.mini-btn {{
  cursor: pointer;
  background: var(--bg);
  color: var(--muted);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 3px 8px;
  font-size: 0.75rem;
  margin-right: 4px;
}}
.mini-btn:hover {{
  color: var(--text);
  border-color: var(--accent);
}}
footer {{
  text-align: center;
  color: var(--muted);
  font-size: 0.8rem;
  margin-top: 26px;
}}
@media print {{
  .controls, .actions-cell, .header-actions button {{ display: none; }}
  body {{ background: #fff; color: #000; }}
  .panel, .kpi-card {{ border-color: #ccc; }}
}}
</style>
</head>
<body>
<div class="container">
  <header class="top-bar">
    <div class="brand">
      <h1>nodewipe Storage Audit Report</h1>
      <p class="subtitle">Root: {root_esc}</p>
    </div>
    <div class="header-actions">
      <span class="meta-pill">Generated: {date_esc}</span>
      <span class="meta-pill">nodewipe v{version}</span>
      <button type="button" id="themeToggle" class="btn">Toggle Theme</button>
    </div>
  </header>

  <section class="kpi-grid">
    <div class="kpi-card">
      <div class="kpi-label">Total Reclaimable</div>
      <div class="kpi-value" id="kpiTotalSize">{total_esc}</div>
    </div>
    <div class="kpi-card">
      <div class="kpi-label">Artifacts Discovered</div>
      <div class="kpi-value" id="kpiCount">{entry_count}</div>
    </div>
    <div class="kpi-card">
      <div class="kpi-label">Ecosystems / Workspaces</div>
      <div class="kpi-value">{eco_count} / {ws_count}</div>
    </div>
    <div class="kpi-card">
      <div class="kpi-label">Oldest Artifact</div>
      <div class="kpi-value">{oldest_str}</div>
    </div>
  </section>

  <div class="charts-grid">
    <section class="panel">
      <h2>Disk Usage by Ecosystem</h2>
      <div class="donut-wrap">
        {donut_svg}
        <div class="legend" id="ecoLegend">
          {legend_html}
        </div>
      </div>
    </section>

    <section class="panel">
      <h2>Artifact Staleness Timeline</h2>
      {age_svg}
    </section>
  </div>

  {stale_section}

  <section class="panel">
    <h2>Discovered Artifacts</h2>
    <div class="controls">
      <input type="search" id="searchInput" placeholder="Filter by path, ecosystem, or git branch..." aria-label="Filter artifacts">
      <select id="ecoFilter" aria-label="Filter by ecosystem">
        {eco_options}
      </select>
      <select id="ageFilter" aria-label="Filter by age bucket">
        {age_options}
      </select>
      <button type="button" id="resetFilters" class="btn">Reset Filters</button>
      <span class="filter-status" id="filterStatus">Showing {entry_count} of {entry_count}</span>
    </div>
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th class="sortable" data-sort="kind_label">Ecosystem</th>
            <th class="sortable" data-sort="path">Path</th>
            <th class="sortable num" data-sort="size_bytes">Size ▼</th>
            <th class="sortable num" data-sort="age_days">Age</th>
            <th class="sortable" data-sort="risk">Risk</th>
            <th>Actions</th>
          </tr>
        </thead>
        <tbody id="artifactBody">
          {ssr_rows}
        </tbody>
      </table>
    </div>
  </section>

  <footer>
    Generated offline by nodewipe v{version} - Zero external network requests.
  </footer>
</div>

<script id="report-data" type="application/json">{json_safe}</script>
<script>
(function() {{
  "use strict";
  const rawEl = document.getElementById("report-data");
  if (!rawEl) return;
  const payload = JSON.parse(rawEl.textContent || "{{}}");
  const entries = Array.isArray(payload.entries) ? payload.entries : [];

  let sortKey = "size_bytes";
  let sortAsc = false;
  let searchQuery = "";
  let selectedEco = "";
  let selectedAge = "";

  const tbody = document.getElementById("artifactBody");
  const searchInput = document.getElementById("searchInput");
  const ecoFilter = document.getElementById("ecoFilter");
  const ageFilter = document.getElementById("ageFilter");
  const resetBtn = document.getElementById("resetFilters");
  const filterStatus = document.getElementById("filterStatus");
  const kpiTotal = document.getElementById("kpiTotalSize");
  const kpiCount = document.getElementById("kpiCount");
  const themeBtn = document.getElementById("themeToggle");

  function formatBytes(bytes) {{
    const units = ["B", "KB", "MB", "GB", "TB"];
    let val = Number(bytes) || 0;
    let idx = 0;
    while (val >= 1024 && idx < units.length - 1) {{
      val /= 1024;
      idx++;
    }}
    return idx === 0 ? val + " B" : val.toFixed(idx >= 3 ? 2 : 1) + " " + units[idx];
  }}

  function copyText(text, btn) {{
    const done = function() {{
      const prev = btn.textContent;
      btn.textContent = "Copied";
      setTimeout(function() {{ btn.textContent = prev; }}, 1200);
    }};
    if (navigator.clipboard && navigator.clipboard.writeText) {{
      navigator.clipboard.writeText(text).then(done).catch(function() {{ fallbackCopy(text, done); }});
    }} else {{
      fallbackCopy(text, done);
    }}
  }}

  function fallbackCopy(text, cb) {{
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    try {{ document.execCommand("copy"); cb(); }} catch (_) {{}}
    document.body.removeChild(ta);
  }}

  function syncChartHighlights() {{
    document.querySelectorAll(".donut-slice, .legend-item").forEach(function(el) {{
      el.classList.toggle("active", selectedEco !== "" && el.getAttribute("data-ecosystem") === selectedEco);
    }});
    document.querySelectorAll(".age-bar-group").forEach(function(el) {{
      el.classList.toggle("active", selectedAge !== "" && el.getAttribute("data-age-bucket") === selectedAge);
    }});
  }}

  function updateSortHeaders() {{
    document.querySelectorAll("th.sortable").forEach(function(th) {{
      const key = th.getAttribute("data-sort");
      const base = th.getAttribute("data-base-label") || th.textContent.replace(/ [▲▼]$/, "");
      th.setAttribute("data-base-label", base);
      th.textContent = key === sortKey ? base + (sortAsc ? " ▲" : " ▼") : base;
    }});
  }}

  function render() {{
    const q = searchQuery.trim().toLowerCase();
    const filtered = entries.filter(function(item) {{
      if (selectedEco && item.kind_label !== selectedEco) return false;
      if (selectedAge && item.age_bucket !== selectedAge) return false;
      if (q) {{
        const haystack = (
          item.path + " " + item.kind_label + " " + (item.git_branch || "")
        ).toLowerCase();
        if (haystack.indexOf(q) === -1) return false;
      }}
      return true;
    }});

    filtered.sort(function(a, b) {{
      let va = a[sortKey];
      let vb = b[sortKey];
      if (sortKey === "age_days") {{
        va = va === null || va === undefined ? -1 : va;
        vb = vb === null || vb === undefined ? -1 : vb;
      }}
      let cmp = 0;
      if (typeof va === "number" && typeof vb === "number") {{
        cmp = va - vb;
      }} else {{
        cmp = String(va || "").localeCompare(String(vb || ""));
      }}
      return sortAsc ? cmp : -cmp;
    }});

    while (tbody.firstChild) {{
      tbody.removeChild(tbody.firstChild);
    }}

    let visibleBytes = 0;
    filtered.forEach(function(item) {{
      visibleBytes += item.size_bytes;
      const tr = document.createElement("tr");

      const tdEco = document.createElement("td");
      const ecoSpan = document.createElement("span");
      ecoSpan.className = "badge-eco";
      ecoSpan.style.setProperty("--eco", item.color);
      ecoSpan.textContent = item.kind_label;
      tdEco.appendChild(ecoSpan);
      tr.appendChild(tdEco);

      const tdPath = document.createElement("td");
      tdPath.className = "mono";
      tdPath.appendChild(document.createTextNode(item.path));
      if (item.git_branch) {{
        const gitSpan = document.createElement("span");
        gitSpan.className = "badge badge-git";
        gitSpan.textContent = item.git_branch + (item.git_dirty ? "*" : "") + (item.git_merged ? " merged" : "");
        tdPath.appendChild(gitSpan);
      }}
      if (item.pinned) {{
        const pinSpan = document.createElement("span");
        pinSpan.className = "badge badge-pin";
        pinSpan.textContent = "PINNED";
        tdPath.appendChild(pinSpan);
      }}
      tr.appendChild(tdPath);

      const tdSize = document.createElement("td");
      tdSize.className = "num";
      tdSize.title = item.size_bytes + " bytes";
      tdSize.textContent = item.size_formatted;
      tr.appendChild(tdSize);

      const tdAge = document.createElement("td");
      tdAge.className = "num";
      tdAge.textContent = item.age_days !== null && item.age_days !== undefined ? item.age_days + "d" : "?";
      tr.appendChild(tdAge);

      const tdRisk = document.createElement("td");
      const riskSpan = document.createElement("span");
      riskSpan.className = "badge badge-risk-" + item.risk;
      riskSpan.textContent = item.risk;
      tdRisk.appendChild(riskSpan);
      tr.appendChild(tdRisk);

      const tdAct = document.createElement("td");
      tdAct.className = "actions-cell";

      const copyPathBtn = document.createElement("button");
      copyPathBtn.type = "button";
      copyPathBtn.className = "mini-btn";
      copyPathBtn.textContent = "Copy Path";
      copyPathBtn.addEventListener("click", function() {{ copyText(item.path, copyPathBtn); }});
      tdAct.appendChild(copyPathBtn);

      const cmd = 'nodewipe delete "' + item.path.replace(/"/g, '\\"') + '" --mode trash --yes';
      const copyCmdBtn = document.createElement("button");
      copyCmdBtn.type = "button";
      copyCmdBtn.className = "mini-btn";
      copyCmdBtn.textContent = "Copy Cmd";
      copyCmdBtn.addEventListener("click", function() {{ copyText(cmd, copyCmdBtn); }});
      tdAct.appendChild(copyCmdBtn);

      tr.appendChild(tdAct);
      tbody.appendChild(tr);
    }});

    if (filtered.length === 0) {{
      const emptyTr = document.createElement("tr");
      const emptyTd = document.createElement("td");
      emptyTd.colSpan = 6;
      emptyTd.style.textAlign = "center";
      emptyTd.style.color = "var(--muted)";
      emptyTd.textContent = "No matching artifacts.";
      emptyTr.appendChild(emptyTd);
      tbody.appendChild(emptyTr);
    }}

    filterStatus.textContent = "Showing " + filtered.length + " of " + entries.length + " (" + formatBytes(visibleBytes) + ")";
    kpiTotal.textContent = formatBytes(visibleBytes);
    kpiCount.textContent = String(filtered.length);
    syncChartHighlights();
    updateSortHeaders();
  }}

  searchInput.addEventListener("input", function() {{
    searchQuery = searchInput.value;
    render();
  }});

  ecoFilter.addEventListener("change", function() {{
    selectedEco = ecoFilter.value;
    render();
  }});

  ageFilter.addEventListener("change", function() {{
    selectedAge = ageFilter.value;
    render();
  }});

  resetBtn.addEventListener("click", function() {{
    searchQuery = "";
    selectedEco = "";
    selectedAge = "";
    searchInput.value = "";
    ecoFilter.value = "";
    ageFilter.value = "";
    render();
  }});

  document.querySelectorAll("th.sortable").forEach(function(th) {{
    th.addEventListener("click", function() {{
      const key = th.getAttribute("data-sort");
      if (sortKey === key) {{
        sortAsc = !sortAsc;
      }} else {{
        sortKey = key;
        sortAsc = key === "path" || key === "kind_label" || key === "risk";
      }}
      render();
    }});
  }});

  document.querySelectorAll(".donut-slice, .legend-item").forEach(function(el) {{
    el.addEventListener("click", function() {{
      const eco = el.getAttribute("data-ecosystem");
      selectedEco = selectedEco === eco ? "" : eco;
      ecoFilter.value = selectedEco;
      render();
    }});
  }});

  document.querySelectorAll(".age-bar-group").forEach(function(el) {{
    el.addEventListener("click", function() {{
      const bucket = el.getAttribute("data-age-bucket");
      selectedAge = selectedAge === bucket ? "" : bucket;
      ageFilter.value = selectedAge;
      render();
    }});
  }});

  themeBtn.addEventListener("click", function() {{
    const html = document.documentElement;
    html.setAttribute("data-theme", html.getAttribute("data-theme") === "dark" ? "light" : "dark");
  }});

  render();
}})();
</script>
</body>
</html>
"#,
        entry_count = payload.entries.len(),
        eco_count = payload.ecosystems.len(),
        ws_count = payload.workspace_count,
    );

    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodewipe_core::{GitStatus, PackageManager};
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn test_escape_html_and_script_json_prevents_xss() {
        let hostile = r#"</script><script>alert("xss")</script>&'test'"#;
        let escaped_html = escape_html(hostile);
        assert!(!escaped_html.contains('<'));
        assert!(!escaped_html.contains('>'));
        assert!(escaped_html.contains("&lt;/script&gt;"));
        assert!(escaped_html.contains("&amp;"));
        assert!(escaped_html.contains("&#39;"));

        let escaped_json = escape_json_for_script(hostile);
        assert!(!escaped_json.contains("</script>"));
        assert!(escaped_json.contains("\\u003c/script\\u003e"));
        assert!(escaped_json.contains("\\u0026"));
    }

    #[test]
    fn test_generate_html_report_self_contained_and_escaped() {
        let now = SystemTime::now();
        let entries = vec![
            Entry {
                path: PathBuf::from("/workspace/app/node_modules"),
                size_bytes: 52_428_800,
                kind: ArtifactKind::NodeModules,
                package_manager: Some(PackageManager::Pnpm),
                last_modified: Some(now - Duration::from_secs(45 * 86400)),
                workspace_root: Some(PathBuf::from("/workspace")),
                git_status: Some(GitStatus {
                    branch: Some("feat/<script>alert(1)</script>".to_string()),
                    is_dirty: true,
                    is_merged: false,
                    last_commit_time: None,
                }),
                custom_rule: None,
            },
            Entry {
                path: PathBuf::from("/workspace/rust-crate/target"),
                size_bytes: 104_857_600,
                kind: ArtifactKind::RustTarget,
                package_manager: None,
                last_modified: Some(now - Duration::from_secs(2 * 86400)),
                workspace_root: Some(PathBuf::from("/workspace")),
                git_status: None,
                custom_rule: None,
            },
        ];

        let stale = vec![StaleProject {
            root: PathBuf::from("/workspace/old-app"),
            artifact: entries[0].clone(),
            stale_days: 120,
            reclaimable_bytes: 52_428_800,
            git_branch: Some("legacy".to_string()),
            is_merged: true,
            is_dirty: false,
        }];

        let html = generate_html_report(Path::new("/workspace"), &entries, &stale)
            .expect("html report generation succeeds");

        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<html lang=\"en\""));
        assert!(html.contains("nodewipe Storage Audit Report"));
        // Must have zero external CDN scripts/links
        assert!(!html.contains("https://cdn."));
        assert!(!html.contains("<link rel=\"stylesheet\" href=\"http"));
        // Must include SVG charts
        assert!(html.contains("class=\"donut-svg\""));
        assert!(html.contains("class=\"age-svg\""));
        // Hostile branch name must be escaped inside embedded JSON
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("\\u003cscript\\u003ealert(1)\\u003c/script\\u003e"));
        // Contains both entries and stale project section
        assert!(html.contains("/workspace/app/node_modules"));
        assert!(html.contains("/workspace/rust-crate/target"));
        assert!(html.contains("Likely-Abandoned Projects (1)"));
    }

    #[test]
    fn test_generate_html_report_empty_entries() {
        let html = generate_html_report(Path::new("/empty"), &[], &[])
            .expect("empty html report succeeds");
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("0 B"));
        assert!(html.contains("Artifacts Discovered"));
    }
}
