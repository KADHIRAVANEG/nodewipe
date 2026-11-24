//! Squarified Treemap Layout Engine and Visual Heatmap for nodewipe TUI.
//!
//! Implements the Bruls-Huizing-van Wijk squarified treemap tiling algorithm,
//! providing proportional area representation of disk storage consumption,
//! 2D spatial keyboard navigation, drill-down into subdirectories, and visual heatmap rendering.

use nodewipe_core::ArtifactKind;
use ratatui::layout::Rect;
use std::path::{Path, PathBuf};

/// A 2D bounding rectangle in terminal coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreemapRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl TreemapRect {
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[allow(dead_code)]
    pub fn area(&self) -> u32 {
        self.width as u32 * self.height as u32
    }

    pub fn center(&self) -> (f64, f64) {
        (
            self.x as f64 + (self.width as f64) / 2.0,
            self.y as f64 + (self.height as f64) / 2.0,
        )
    }

    pub fn to_ratatui(self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }

    pub fn from_ratatui(r: Rect) -> Self {
        Self {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

/// A weighted item to be laid out in the treemap.
#[derive(Debug, Clone, PartialEq)]
pub struct TreemapItem {
    pub id: String,
    pub label: String,
    pub secondary_label: String,
    pub size_bytes: u64,
    pub kind: Option<ArtifactKind>,
    pub entry_index: Option<usize>,
    pub path: PathBuf,
    pub is_dir: bool,
    /// When true, this item represents an aggregated group of tail items.
    pub is_aggregated: bool,
    /// Aggregated children if any.
    pub aggregated_items: Vec<TreemapItem>,
}

impl TreemapItem {
    pub fn new(
        label: impl Into<String>,
        size_bytes: u64,
        kind: Option<ArtifactKind>,
        entry_index: Option<usize>,
        path: PathBuf,
    ) -> Self {
        let label_str = label.into();
        Self {
            id: path.to_string_lossy().to_string(),
            secondary_label: path.to_string_lossy().to_string(),
            label: label_str,
            size_bytes,
            kind,
            entry_index,
            path,
            is_dir: true,
            is_aggregated: false,
            aggregated_items: Vec::new(),
        }
    }
}

/// A positioned tile in the rendered treemap.
#[derive(Debug, Clone, PartialEq)]
pub struct TreemapTile {
    pub rect: TreemapRect,
    pub item_index: usize,
    pub label: String,
    pub secondary_label: String,
    pub size_bytes: u64,
    pub percentage: f64,
    pub kind: Option<ArtifactKind>,
    pub entry_index: Option<usize>,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_aggregated: bool,
}

/// 2D directional navigation for arrow keys and hjkl.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction2D {
    Up,
    Down,
    Left,
    Right,
}

/// Saved zoom state for hierarchical drilldown and zoom out.
#[derive(Debug, Clone)]
pub struct TreemapZoomState {
    pub title: String,
    #[allow(dead_code)]
    pub path: PathBuf,
    #[allow(dead_code)]
    pub parent_entry_index: Option<usize>,
    pub items: Vec<TreemapItem>,
    pub cursor: usize,
}

// ── Squarified Treemap Layout Algorithm ──────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct FloatRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl FloatRect {
    fn shorter_side(&self) -> f64 {
        self.w.min(self.h)
    }
}

/// Computes the worst aspect ratio for a row of normalized item areas along a given side length.
fn worst_aspect_ratio(row: &[f64], side: f64) -> f64 {
    if row.is_empty() || side <= 0.0 {
        return f64::MAX;
    }
    let s: f64 = row.iter().sum();
    if s <= 0.0 {
        return f64::MAX;
    }
    let s_sq = s * s;
    let side_sq = side * side;

    let mut worst = 1.0f64;
    for &r in row {
        if r <= 0.0 {
            continue;
        }
        let a = (side_sq * r) / s_sq;
        let b = s_sq / (side_sq * r);
        let ar = a.max(b);
        if ar > worst {
            worst = ar;
        }
    }
    worst
}

struct LayoutItem {
    entry_index: Option<usize>,
    label: String,
    secondary_label: String,
    size_bytes: u64,
    kind: Option<ArtifactKind>,
    path: PathBuf,
    is_dir: bool,
    is_aggregated: bool,
}

/// Tiling context that subdivides a bounding rectangle using the squarified algorithm.
pub struct TreemapEngine;

impl TreemapEngine {
    /// Computes squarified tile positions for a list of items within a container rectangle.
    pub fn layout(container: TreemapRect, items: &[TreemapItem]) -> Vec<TreemapTile> {
        if items.is_empty() || container.width == 0 || container.height == 0 {
            return Vec::new();
        }

        let total_size: u64 = items.iter().map(|it| it.size_bytes).sum();
        let total_area = (container.width as f64) * (container.height as f64);

        if total_size == 0 || total_area <= 0.0 {
            // Distribute items evenly if all have size 0
            return Self::layout_equal(container, items);
        }

        // Sort items by size descending
        let mut indexed_items: Vec<(usize, &TreemapItem)> = items.iter().enumerate().collect();
        indexed_items.sort_by_key(|a| std::cmp::Reverse(a.1.size_bytes));

        // Aggregate tail items if there are too many items to render cleanly
        // A terminal can comfortably display around 30-40 interactive tiles.
        let max_tiles = 40.min(indexed_items.len());
        let (head, tail) = indexed_items.split_at(max_tiles);

        let mut layout_items: Vec<LayoutItem> = Vec::new();

        for &(orig_idx, it) in head {
            layout_items.push(LayoutItem {
                entry_index: Some(orig_idx),
                label: it.label.clone(),
                secondary_label: it.secondary_label.clone(),
                size_bytes: it.size_bytes,
                kind: it.kind,
                path: it.path.clone(),
                is_dir: it.is_dir,
                is_aggregated: it.is_aggregated,
            });
        }

        if !tail.is_empty() {
            let tail_size: u64 = tail.iter().map(|(_, it)| it.size_bytes).sum();
            let tail_count = tail.len();
            layout_items.push(LayoutItem {
                entry_index: None,
                label: format!("+{} other items", tail_count),
                secondary_label: format!("{} smaller items aggregated", tail_count),
                size_bytes: tail_size,
                kind: None,
                path: container_placeholder_path(&container),
                is_dir: true,
                is_aggregated: true,
            });
        }

        let num_items = layout_items.len();
        let normalized_areas: Vec<f64> = layout_items
            .iter()
            .map(|li| ((li.size_bytes as f64) / (total_size as f64) * total_area).max(0.001))
            .collect();

        let mut float_rects: Vec<FloatRect> = vec![
            FloatRect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0
            };
            num_items
        ];

        let mut remaining_rect = FloatRect {
            x: container.x as f64,
            y: container.y as f64,
            w: container.width as f64,
            h: container.height as f64,
        };

        let mut current_row: Vec<(usize, f64)> = Vec::new();

        for (i, &area) in normalized_areas.iter().enumerate() {
            let side = remaining_rect.shorter_side();
            if side <= 0.001 {
                break;
            }

            let mut proposed_row: Vec<f64> = current_row.iter().map(|(_, a)| *a).collect();
            proposed_row.push(area);

            let current_worst = if current_row.is_empty() {
                f64::MAX
            } else {
                worst_aspect_ratio(
                    &current_row.iter().map(|(_, a)| *a).collect::<Vec<_>>(),
                    side,
                )
            };

            let proposed_worst = worst_aspect_ratio(&proposed_row, side);

            if current_row.is_empty() || proposed_worst <= current_worst {
                current_row.push((i, area));
            } else {
                // Layout current row and advance remaining rectangle
                Self::layout_row(&current_row, &mut remaining_rect, &mut float_rects);
                current_row.clear();
                current_row.push((i, area));
            }
        }

        if !current_row.is_empty() {
            Self::layout_row(&current_row, &mut remaining_rect, &mut float_rects);
        }

        // Discretize float_rects to terminal grid coordinates with bounds clamping and gap prevention
        let mut tiles: Vec<TreemapTile> = Vec::with_capacity(num_items);

        let max_x = container.x + container.width;
        let max_y = container.y + container.height;

        for (i, fr) in float_rects.iter().enumerate() {
            let li = &layout_items[i];

            let x1 = (fr.x.round() as u16).max(container.x).min(max_x);
            let y1 = (fr.y.round() as u16).max(container.y).min(max_y);
            let x2 = ((fr.x + fr.w).round() as u16).max(x1).min(max_x);
            let y2 = ((fr.y + fr.h).round() as u16).max(y1).min(max_y);

            let w = (x2.saturating_sub(x1)).max(if container.width > 0 { 1 } else { 0 });
            let h = (y2.saturating_sub(y1)).max(if container.height > 0 { 1 } else { 0 });

            let final_w = w.min(max_x.saturating_sub(x1));
            let final_h = h.min(max_y.saturating_sub(y1));

            let pct = if total_size > 0 {
                (li.size_bytes as f64 / total_size as f64) * 100.0
            } else {
                0.0
            };

            let item_index = li.entry_index.unwrap_or(i);

            tiles.push(TreemapTile {
                rect: TreemapRect::new(x1, y1, final_w, final_h),
                item_index,
                label: li.label.clone(),
                secondary_label: li.secondary_label.clone(),
                size_bytes: li.size_bytes,
                percentage: pct,
                kind: li.kind,
                entry_index: li.entry_index,
                path: li.path.clone(),
                is_dir: li.is_dir,
                is_aggregated: li.is_aggregated,
            });
        }

        tiles
    }

    fn layout_row(row: &[(usize, f64)], remaining: &mut FloatRect, out: &mut [FloatRect]) {
        if row.is_empty() || remaining.w <= 0.0 || remaining.h <= 0.0 {
            return;
        }

        let sum_area: f64 = row.iter().map(|(_, a)| *a).sum();
        let side = remaining.shorter_side();
        let thickness = sum_area / side;

        if remaining.w <= remaining.h {
            // Horizontal row across remaining.w
            let mut current_x = remaining.x;
            for &(idx, area) in row {
                let item_w = if sum_area > 0.0 {
                    (area / sum_area) * remaining.w
                } else {
                    remaining.w / row.len() as f64
                };
                out[idx] = FloatRect {
                    x: current_x,
                    y: remaining.y,
                    w: item_w,
                    h: thickness,
                };
                current_x += item_w;
            }
            remaining.y += thickness;
            remaining.h = (remaining.h - thickness).max(0.0);
        } else {
            // Vertical column down remaining.h
            let mut current_y = remaining.y;
            for &(idx, area) in row {
                let item_h = if sum_area > 0.0 {
                    (area / sum_area) * remaining.h
                } else {
                    remaining.h / row.len() as f64
                };
                out[idx] = FloatRect {
                    x: remaining.x,
                    y: current_y,
                    w: thickness,
                    h: item_h,
                };
                current_y += item_h;
            }
            remaining.x += thickness;
            remaining.w = (remaining.w - thickness).max(0.0);
        }
    }

    fn layout_equal(container: TreemapRect, items: &[TreemapItem]) -> Vec<TreemapTile> {
        let n = items.len();
        if n == 0 {
            return Vec::new();
        }
        let cols = (n as f64).sqrt().ceil() as u16;
        let rows = ((n as f64) / (cols as f64)).ceil() as u16;
        let cell_w = (container.width / cols).max(1);
        let cell_h = (container.height / rows).max(1);

        let mut tiles = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let r = (i as u16) / cols;
            let c = (i as u16) % cols;
            let x = container.x + c * cell_w;
            let y = container.y + r * cell_h;
            let w = cell_w.min(container.x + container.width - x);
            let h = cell_h.min(container.y + container.height - y);
            tiles.push(TreemapTile {
                rect: TreemapRect::new(x, y, w, h),
                item_index: i,
                label: item.label.clone(),
                secondary_label: item.secondary_label.clone(),
                size_bytes: item.size_bytes,
                percentage: if n > 0 { 100.0 / n as f64 } else { 0.0 },
                kind: item.kind,
                entry_index: item.entry_index,
                path: item.path.clone(),
                is_dir: item.is_dir,
                is_aggregated: item.is_aggregated,
            });
        }
        tiles
    }

    /// Finds the best adjacent tile in the given 2D direction based on Euclidean geometric alignment.
    pub fn navigate(current_index: usize, tiles: &[TreemapTile], dir: Direction2D) -> usize {
        if tiles.is_empty() {
            return 0;
        }
        if current_index >= tiles.len() {
            return 0;
        }

        let curr = &tiles[current_index];
        let (cx, cy) = curr.rect.center();

        let mut best_index = current_index;
        let mut min_score = f64::MAX;

        for (i, candidate) in tiles.iter().enumerate() {
            if i == current_index {
                continue;
            }
            let (tx, ty) = candidate.rect.center();

            match dir {
                Direction2D::Right => {
                    if tx > cx {
                        let dx = tx - cx;
                        let dy = (ty - cy).abs();
                        let score = dx + 2.5 * dy;
                        if score < min_score {
                            min_score = score;
                            best_index = i;
                        }
                    }
                }
                Direction2D::Left => {
                    if tx < cx {
                        let dx = cx - tx;
                        let dy = (ty - cy).abs();
                        let score = dx + 2.5 * dy;
                        if score < min_score {
                            min_score = score;
                            best_index = i;
                        }
                    }
                }
                Direction2D::Down => {
                    if ty > cy {
                        let dy = ty - cy;
                        let dx = (tx - cx).abs();
                        let score = 2.5 * dx + dy;
                        if score < min_score {
                            min_score = score;
                            best_index = i;
                        }
                    }
                }
                Direction2D::Up => {
                    if ty < cy {
                        let dy = cy - ty;
                        let dx = (tx - cx).abs();
                        let score = 2.5 * dx + dy;
                        if score < min_score {
                            min_score = score;
                            best_index = i;
                        }
                    }
                }
            }
        }

        // If no candidate was found strictly in that half-plane, wrap around or cycle
        if best_index == current_index {
            match dir {
                Direction2D::Right | Direction2D::Down => (current_index + 1) % tiles.len(),
                Direction2D::Left | Direction2D::Up => {
                    if current_index == 0 {
                        tiles.len() - 1
                    } else {
                        current_index - 1
                    }
                }
            }
        } else {
            best_index
        }
    }
}

fn container_placeholder_path(container: &TreemapRect) -> PathBuf {
    PathBuf::from(format!("aggregated-{}-{}", container.x, container.y))
}

/// Inspects the direct children of a directory on disk to build a child treemap hierarchy.
pub fn inspect_dir_for_treemap(path: &Path) -> Vec<TreemapItem> {
    let mut items = Vec::new();
    let entries = match std::fs::read_dir(path) {
        Ok(read_dir) => read_dir,
        Err(_) => return items,
    };

    for entry_res in entries.flatten() {
        let child_path = entry_res.path();
        let name = entry_res.file_name().to_string_lossy().to_string();
        let metadata = match entry_res.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let is_dir = metadata.is_dir();
        let size_bytes = if is_dir {
            // Shallow walk calculation with a reasonable budget
            compute_dir_size_capped(&child_path, 20_000)
        } else {
            metadata.len()
        };

        items.push(TreemapItem {
            id: child_path.to_string_lossy().to_string(),
            label: name.clone(),
            secondary_label: child_path.to_string_lossy().to_string(),
            size_bytes,
            kind: None,
            entry_index: None,
            path: child_path,
            is_dir,
            is_aggregated: false,
            aggregated_items: Vec::new(),
        });
    }

    // Sort descending by size
    items.sort_by_key(|a| std::cmp::Reverse(a.size_bytes));
    items
}

/// Computes directory size with a file count cap to guarantee instant response times.
fn compute_dir_size_capped(path: &Path, max_entries: usize) -> u64 {
    let mut total = 0u64;
    let mut count = 0;
    for entry in walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if let Ok(meta) = entry.metadata()
            && meta.is_file()
        {
            total += meta.len();
        }
        count += 1;
        if count >= max_entries {
            break;
        }
    }
    total
}

// ── Tests ───────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_treemap_rect_area_and_center() {
        let r = TreemapRect::new(10, 20, 30, 40);
        assert_eq!(r.area(), 1200);
        assert_eq!(r.center(), (25.0, 40.0));
    }

    #[test]
    fn test_empty_layout() {
        let container = TreemapRect::new(0, 0, 100, 50);
        let tiles = TreemapEngine::layout(container, &[]);
        assert!(tiles.is_empty());
    }

    #[test]
    fn test_single_item_layout() {
        let container = TreemapRect::new(0, 0, 80, 24);
        let items = vec![TreemapItem::new(
            "target",
            1_000_000,
            None,
            Some(0),
            PathBuf::from("/target"),
        )];
        let tiles = TreemapEngine::layout(container, &items);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].rect.x, 0);
        assert_eq!(tiles[0].rect.y, 0);
        assert_eq!(tiles[0].rect.width, 80);
        assert_eq!(tiles[0].rect.height, 24);
        assert_eq!(tiles[0].percentage, 100.0);
    }

    #[test]
    fn test_squarified_proportional_and_no_overlap() {
        let container = TreemapRect::new(0, 0, 100, 40);
        let items = vec![
            TreemapItem::new(
                "node_modules",
                600,
                None,
                Some(0),
                PathBuf::from("/p1/node_modules"),
            ),
            TreemapItem::new("target", 200, None, Some(1), PathBuf::from("/p2/target")),
            TreemapItem::new(".venv", 100, None, Some(2), PathBuf::from("/p3/.venv")),
            TreemapItem::new("build", 100, None, Some(3), PathBuf::from("/p4/build")),
        ];

        let tiles = TreemapEngine::layout(container, &items);
        assert_eq!(tiles.len(), 4);

        // Verify all tiles fit within container bounds
        for t in &tiles {
            assert!(t.rect.x >= container.x);
            assert!(t.rect.y >= container.y);
            assert!(t.rect.x + t.rect.width <= container.x + container.width);
            assert!(t.rect.y + t.rect.height <= container.y + container.height);
            assert!(t.rect.width > 0);
            assert!(t.rect.height > 0);
        }

        // Verify largest item gets the largest area
        let area0 = tiles[0].rect.area();
        let area1 = tiles[1].rect.area();
        assert!(
            area0 >= area1,
            "Largest item area ({area0}) should be >= second item ({area1})"
        );
    }

    #[test]
    fn test_directional_2d_navigation() {
        // Arrange 4 tiles in a 2x2 grid
        let tiles = vec![
            TreemapTile {
                rect: TreemapRect::new(0, 0, 50, 20),
                item_index: 0,
                label: "Top-Left".into(),
                secondary_label: "".into(),
                size_bytes: 100,
                percentage: 25.0,
                kind: None,
                entry_index: Some(0),
                path: PathBuf::from("/tl"),
                is_dir: true,
                is_aggregated: false,
            },
            TreemapTile {
                rect: TreemapRect::new(50, 0, 50, 20),
                item_index: 1,
                label: "Top-Right".into(),
                secondary_label: "".into(),
                size_bytes: 100,
                percentage: 25.0,
                kind: None,
                entry_index: Some(1),
                path: PathBuf::from("/tr"),
                is_dir: true,
                is_aggregated: false,
            },
            TreemapTile {
                rect: TreemapRect::new(0, 20, 50, 20),
                item_index: 2,
                label: "Bottom-Left".into(),
                secondary_label: "".into(),
                size_bytes: 100,
                percentage: 25.0,
                kind: None,
                entry_index: Some(2),
                path: PathBuf::from("/bl"),
                is_dir: true,
                is_aggregated: false,
            },
            TreemapTile {
                rect: TreemapRect::new(50, 20, 50, 20),
                item_index: 3,
                label: "Bottom-Right".into(),
                secondary_label: "".into(),
                size_bytes: 100,
                percentage: 25.0,
                kind: None,
                entry_index: Some(3),
                path: PathBuf::from("/br"),
                is_dir: true,
                is_aggregated: false,
            },
        ];

        // Navigating from Top-Left (0):
        // Right should go to Top-Right (1)
        assert_eq!(TreemapEngine::navigate(0, &tiles, Direction2D::Right), 1);
        // Down should go to Bottom-Left (2)
        assert_eq!(TreemapEngine::navigate(0, &tiles, Direction2D::Down), 2);

        // Navigating from Bottom-Right (3):
        // Left should go to Bottom-Left (2)
        assert_eq!(TreemapEngine::navigate(3, &tiles, Direction2D::Left), 2);
        // Up should go to Top-Right (1)
        assert_eq!(TreemapEngine::navigate(3, &tiles, Direction2D::Up), 1);
    }
}
