use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// A node in the artifact directory storage drill-down tree.
#[derive(Debug, Clone, Serialize)]
pub struct TreeNode {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub formatted_size: String,
    pub percentage: f64,
    pub is_dir: bool,
    pub children: Vec<TreeNode>,
    pub other_items_count: usize,
    pub other_items_bytes: u64,
}

/// The complete inspected artifact tree.
#[derive(Debug, Clone, Serialize)]
pub struct ArtifactTree {
    pub root_path: PathBuf,
    pub root_name: String,
    pub total_size_bytes: u64,
    pub formatted_total_size: String,
    pub children: Vec<TreeNode>,
}

/// Inspects the internal storage tree of an artifact directory.
///
/// Discovers immediate children, sorts them by size descending, and recurses
/// up to `max_depth`. At each level, at most `limit` largest children are expanded,
/// with the rest summarized to prevent terminal clutter.
pub fn inspect_artifact_tree(
    path: &Path,
    max_depth: usize,
    limit: usize,
) -> std::io::Result<ArtifactTree> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let root_name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".")
        .to_string();

    let total_size = compute_dir_size(&canonical);
    let children = build_children(&canonical, total_size, 1, max_depth, limit);

    Ok(ArtifactTree {
        root_path: canonical,
        root_name,
        total_size_bytes: total_size,
        formatted_total_size: crate::reclaim::format_bytes(total_size),
        children,
    })
}

fn build_children(
    dir: &Path,
    parent_size: u64,
    current_depth: usize,
    max_depth: usize,
    limit: usize,
) -> Vec<TreeNode> {
    if current_depth > max_depth {
        return Vec::new();
    }

    let read_res = fs::read_dir(dir);
    let entries = match read_res {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };

    let mut nodes: Vec<(TreeNode, u64)> = Vec::new();

    for entry in entries.flatten() {
        let entry_path = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let name = entry.file_name().to_str().unwrap_or("unknown").to_string();
        let is_dir = meta.is_dir();

        let size = if is_dir {
            compute_dir_size(&entry_path)
        } else {
            meta.len()
        };

        let percentage = if parent_size > 0 {
            (size as f64 / parent_size as f64) * 100.0
        } else {
            0.0
        };

        let child_nodes = if is_dir && current_depth < max_depth {
            build_children(&entry_path, size, current_depth + 1, max_depth, limit)
        } else {
            Vec::new()
        };

        let node = TreeNode {
            name,
            path: entry_path,
            size_bytes: size,
            formatted_size: crate::reclaim::format_bytes(size),
            percentage,
            is_dir,
            children: child_nodes,
            other_items_count: 0,
            other_items_bytes: 0,
        };

        nodes.push((node, size));
    }

    // Sort descending by size
    nodes.sort_by_key(|(_, s)| std::cmp::Reverse(*s));

    if nodes.len() > limit {
        let (top, rest) = nodes.split_at_mut(limit);
        let other_count = rest.len();
        let other_bytes: u64 = rest.iter().map(|(_, s)| *s).sum();

        let mut result: Vec<TreeNode> = top.iter().map(|(n, _)| n.clone()).collect();
        if other_count > 0
            && let Some(last) = result.last_mut()
        {
            last.other_items_count = other_count;
            last.other_items_bytes = other_bytes;
        }
        result
    } else {
        nodes.into_iter().map(|(n, _)| n).collect()
    }
}

/// Recursively sums the physical block sizes of a directory.
fn compute_dir_size(dir: &Path) -> u64 {
    crate::scanner::dir_size(dir)
}

impl ArtifactTree {
    /// Renders the tree into a styled terminal string using box drawing characters
    /// and official Nerd Font vector icons.
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            " {} {}/ ({})\n",
            crate::model::icons::FOLDER,
            self.root_name,
            self.formatted_total_size
        ));

        let len = self.children.len();
        for (i, child) in self.children.iter().enumerate() {
            let is_last = i == len - 1;
            render_node(child, "", is_last, &mut out);
        }

        out
    }
}

fn render_node(node: &TreeNode, prefix: &str, is_last: bool, out: &mut String) {
    let connector = if is_last { "└── " } else { "├── " };
    let icon = if node.is_dir {
        crate::model::icons::FOLDER
    } else {
        "\u{f016}" //  file
    };

    let dir_slash = if node.is_dir { "/" } else { "" };
    out.push_str(&format!(
        "{prefix}{connector}{icon} {}{dir_slash} ({} · {:.1}%)\n",
        node.name, node.formatted_size, node.percentage
    ));

    let child_prefix = format!("{prefix}{}", if is_last { "    " } else { "│   " });
    let child_len = node.children.len();

    for (j, child) in node.children.iter().enumerate() {
        let child_last = j == child_len - 1 && node.other_items_count == 0;
        render_node(child, &child_prefix, child_last, out);
    }

    if node.other_items_count > 0 {
        let other_conn = "└── ";
        out.push_str(&format!(
            "{child_prefix}{other_conn}… ({} other items, {})\n",
            node.other_items_count,
            crate::reclaim::format_bytes(node.other_items_bytes)
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_inspect_artifact_tree_structure() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let sub1 = root.join("debug");
        let sub2 = root.join("release");
        fs::create_dir_all(&sub1).unwrap();
        fs::create_dir_all(&sub2).unwrap();

        fs::write(sub1.join("app.o"), vec![0u8; 10000]).unwrap();
        fs::write(sub2.join("app"), vec![0u8; 20000]).unwrap();

        let tree = inspect_artifact_tree(root, 2, 5).unwrap();
        assert_eq!(tree.children.len(), 2);
        // Larger child (release) should be first
        assert_eq!(tree.children[0].name, "release");
        assert_eq!(tree.children[1].name, "debug");

        let rendered = tree.render_text();
        assert!(rendered.contains("release/"));
        assert!(rendered.contains("debug/"));
    }
}
