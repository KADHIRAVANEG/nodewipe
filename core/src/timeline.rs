use crate::model::{ArtifactKind, Entry};
use crate::stale::youngest_source_file_mtime;
use serde::Serialize;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Timeline status classification of an artifact relative to its source code and git history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineVerdict {
    /// Source code was modified AFTER the artifact was built.
    /// Rebuilding would happen on next compilation anyway, so the current build output is redundant.
    SourceNewerThanBuild,
    /// Build output is newer than or matches source code.
    /// Build is fresh; deleting will save space but require full rebuild next run.
    BuildNewerThanSource,
    /// Uncommitted changes or source modified today.
    ActiveWorkInProgress,
    /// No source code found under the project root.
    /// Likely an orphaned artifact from a deleted or relocated project.
    OrphanedBuild,
    /// Both source code and build output untouched for 90+ days.
    Abandoned,
}

impl TimelineVerdict {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SourceNewerThanBuild => "Stale Build (Source Modified Since)",
            Self::BuildNewerThanSource => "Fresh Build (Matches Source)",
            Self::ActiveWorkInProgress => "Active Development (Work In Progress)",
            Self::OrphanedBuild => "Orphaned Artifact (Source Code Missing)",
            Self::Abandoned => "Abandoned Project (Inactive >90d)",
        }
    }
}

/// Project timeline comparing source, build artifact, and git commit activity.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectTimeline {
    pub project_root: PathBuf,
    pub artifact_path: PathBuf,
    pub artifact_kind: ArtifactKind,
    pub source_age_days: Option<u64>,
    pub artifact_age_days: Option<u64>,
    pub git_commit_age_days: Option<u64>,
    pub git_branch: Option<String>,
    pub git_is_dirty: bool,
    pub git_is_merged: bool,
    pub verdict: TimelineVerdict,
    pub summary: String,
    pub recommendation: String,
}

/// Analyzes the timeline relationship between source code, git repository history, and build artifact.
pub fn inspect_timeline(entry: &Entry) -> ProjectTimeline {
    let now = SystemTime::now();
    let canonical = entry
        .path
        .canonicalize()
        .unwrap_or_else(|_| entry.path.clone());
    let root = if canonical.is_dir() && !crate::stale::is_known_artifact_dir(&canonical) {
        canonical.clone()
    } else {
        canonical
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| canonical.clone())
    };

    let youngest_source = youngest_source_file_mtime(&root, &entry.path);
    let artifact_mtime = entry.last_modified;

    let source_age_days = youngest_source
        .and_then(|t| now.duration_since(t).ok())
        .map(|d| d.as_secs() / 86400);

    let artifact_age_days = artifact_mtime
        .and_then(|t| now.duration_since(t).ok())
        .map(|d| d.as_secs() / 86400);

    let (git_branch, git_is_dirty, git_is_merged, git_commit_age_days) = match &entry.git_status {
        Some(g) => {
            let commit_age = g
                .last_commit_time
                .map(|secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
                .and_then(|t| now.duration_since(t).ok())
                .map(|d| d.as_secs() / 86400);
            (g.branch.clone(), g.is_dirty, g.is_merged, commit_age)
        }
        None => (None, false, false, None),
    };

    let (verdict, summary, recommendation) = evaluate_verdict(
        youngest_source,
        artifact_mtime,
        git_is_dirty,
        git_is_merged,
        source_age_days,
        artifact_age_days,
    );

    ProjectTimeline {
        project_root: root,
        artifact_path: entry.path.clone(),
        artifact_kind: entry.kind,
        source_age_days,
        artifact_age_days,
        git_commit_age_days,
        git_branch,
        git_is_dirty,
        git_is_merged,
        verdict,
        summary,
        recommendation,
    }
}

fn evaluate_verdict(
    source_mtime: Option<SystemTime>,
    artifact_mtime: Option<SystemTime>,
    is_dirty: bool,
    is_merged: bool,
    source_age_days: Option<u64>,
    artifact_age_days: Option<u64>,
) -> (TimelineVerdict, String, String) {
    if source_mtime.is_none() {
        return (
            TimelineVerdict::OrphanedBuild,
            "No source code found in the project root directory.".to_string(),
            "High confidence safe to delete: the project source is gone or relocated.".to_string(),
        );
    }

    if is_dirty {
        return (
            TimelineVerdict::ActiveWorkInProgress,
            "Uncommitted edits in repository detected.".to_string(),
            "Proceed with care: working tree contains uncommitted changes.".to_string(),
        );
    }

    if is_merged && source_age_days.is_some_and(|d| d >= 30) {
        return (
            TimelineVerdict::Abandoned,
            "Branch is merged into upstream and source code has been untouched for 30+ days."
                .to_string(),
            "Prime candidate for deletion: work on this branch is already merged.".to_string(),
        );
    }

    if source_age_days.is_some_and(|d| d >= 90) && artifact_age_days.is_some_and(|d| d >= 90) {
        return (
            TimelineVerdict::Abandoned,
            "Both source files and build artifact have been untouched for 90+ days.".to_string(),
            "Safe to reclaim: project appears dormant or abandoned.".to_string(),
        );
    }

    match (source_mtime, artifact_mtime) {
        (Some(src), Some(art)) => {
            if src > art {
                (
                    TimelineVerdict::SourceNewerThanBuild,
                    "Source files were modified after this artifact was created. The build is already out of date.".to_string(),
                    "Safe to delete: compiler/bundler will rebuild anyway on next run.".to_string(),
                )
            } else if source_age_days == Some(0) && artifact_age_days == Some(0) {
                (
                    TimelineVerdict::ActiveWorkInProgress,
                    "Source files and artifact modified today (actively building).".to_string(),
                    "Active project in development today.".to_string(),
                )
            } else {
                (
                    TimelineVerdict::BuildNewerThanSource,
                    "Build output is newer than source code. Current build is up to date.".to_string(),
                    "Deleting will save disk space, but will require re-compiling next time you run.".to_string(),
                )
            }
        }
        _ => (
            TimelineVerdict::BuildNewerThanSource,
            "Timeline timestamps could not be compared precisely.".to_string(),
            "Rebuilding available via detected toolchain.".to_string(),
        ),
    }
}

/// Formats the project timeline into human-readable terminal output.
pub fn format_timeline_text(timeline: &ProjectTimeline) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "\n {} Project Timeline: {}\n",
        crate::model::icons::STATS,
        timeline.project_root.display()
    ));
    out.push_str(&format!(" {}\n", "─".repeat(70)));

    let src_str = timeline
        .source_age_days
        .map(|d| format!("{d} days ago"))
        .unwrap_or_else(|| "Not found / inaccessible".to_string());
    let art_str = timeline
        .artifact_age_days
        .map(|d| format!("{d} days ago"))
        .unwrap_or_else(|| "Unknown".to_string());
    let git_str = match &timeline.git_branch {
        Some(b) => {
            let dirty = if timeline.git_is_dirty {
                " (uncommitted changes)"
            } else {
                " (clean)"
            };
            let merged = if timeline.git_is_merged {
                " [merged]"
            } else {
                ""
            };
            let age = timeline
                .git_commit_age_days
                .map(|d| format!(", commit {d}d ago"))
                .unwrap_or_default();
            format!(
                "{} {b}{dirty}{merged}{age}",
                crate::model::icons::GIT_BRANCH
            )
        }
        None => "No git repository".to_string(),
    };

    out.push_str(&format!(" • Source Code:    {src_str}\n"));
    out.push_str(&format!(
        " • Build Output:   {} ({art_str})\n",
        timeline.artifact_path.display()
    ));
    out.push_str(&format!(" • Git Status:     {git_str}\n"));
    out.push_str(&format!(
        " • Verdict:        {}\n",
        timeline.verdict.label()
    ));
    out.push_str(&format!("   Summary:        {}\n", timeline.summary));
    out.push_str(&format!("   Action:         {}\n", timeline.recommendation));
    out.push_str(&format!(" {}\n", "─".repeat(70)));

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ArtifactKind;
    use tempfile::TempDir;

    #[test]
    fn test_timeline_source_newer_than_build() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let target_dir = root.join("target");
        std::fs::create_dir_all(&target_dir).unwrap();

        let old_time = SystemTime::now() - Duration::from_secs(86400 * 10);

        // Source file modified just now (newer than old_time)
        let src_file = root.join("main.rs");
        std::fs::write(&src_file, "fn main() {}").unwrap();

        let entry = Entry {
            path: target_dir,
            size_bytes: 1024 * 1024,
            kind: ArtifactKind::RustTarget,
            package_manager: None,
            last_modified: Some(old_time), // build was 10 days ago
            workspace_root: None,
            git_status: None,
        };

        let timeline = inspect_timeline(&entry);
        assert_eq!(timeline.verdict, TimelineVerdict::SourceNewerThanBuild);
    }
}
