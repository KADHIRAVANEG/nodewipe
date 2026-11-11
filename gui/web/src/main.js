const { invoke } = window.__TAURI__.core;

let flatEntries = [];
let groups = [];
let viewMode = "flat";
let sortKey = "size";
let sortDir = "desc";
let filterText = "";
const selected = new Set();
const collapsedGroups = new Set();
const excludedKinds = new Set();

const rootInput = document.getElementById("root-input");
const searchInput = document.getElementById("search-input");
const scanBtn = document.getElementById("scan-btn");
const resultsBody = document.getElementById("results-body");
const summary = document.getElementById("summary");
const freedTotal = document.getElementById("freed-total");
const trashBtn = document.getElementById("trash-btn");
const archiveBtn = document.getElementById("archive-btn");
const permanentBtn = document.getElementById("permanent-btn");
const selectAll = document.getElementById("select-all");
const spinner = document.getElementById("spinner");
const emptyState = document.getElementById("empty-state");
const resultsTable = document.getElementById("results");
const viewFlatBtn = document.getElementById("view-flat");
const viewGroupedBtn = document.getElementById("view-grouped");
const modalBackdrop = document.getElementById("modal-backdrop");
const modalBody = document.getElementById("modal-body");
const modalCancel = document.getElementById("modal-cancel");
const modalConfirm = document.getElementById("modal-confirm");
const toastStack = document.getElementById("toast-stack");
const typeChips = document.getElementById("type-chips");

// ── Tab switching ─────────────────────────────────────────────────────────────

document.querySelectorAll(".tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab-btn").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".tab-panel").forEach((p) => p.classList.add("hidden"));
    btn.classList.add("active");
    document.getElementById(`tab-${btn.dataset.tab}`).classList.remove("hidden");

    // Auto-load stats when switching to that tab.
    if (btn.dataset.tab === "stats") loadStats();
  });
});

function escapeHtml(str) {
  if (str == null) return "";
  return String(str)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

function renderChips() {
  typeChips.innerHTML = "";
  Object.entries(KIND_LABELS).forEach(([kind, label]) => {
    const chip = document.createElement("button");
    chip.className = `chip ${excludedKinds.has(kind) ? "" : "active"}`;
    chip.textContent = label;
    chip.addEventListener("click", () => {
      if (excludedKinds.has(kind)) excludedKinds.delete(kind);
      else excludedKinds.add(kind);
      renderChips();
      render();
    });
    typeChips.appendChild(chip);
  });
}

function humanSize(bytes) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let size = bytes;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size.toFixed(2)} ${units[unit]}`;
}

const KIND_LABELS = {
  node_modules: "node_modules",
  python_venv: "Python venv",
  python_pycache: "__pycache__",
  python_pytest_cache: ".pytest_cache",
  python_mypy_cache: ".mypy_cache",
  python_ruff_cache: ".ruff_cache",
  rust_target: "Cargo target",
  java_maven_target: "Maven target",
  java_gradle_build: "Gradle build",
  go_mod_cache: "Go mod cache",
  go_build_cache: "Go build cache",
  php_vendor: "PHP vendor",
  dotnet_bin: ".NET bin/",
  dotnet_obj: ".NET obj/",
  next_cache: "Next.js cache",
  turbo_cache: "Turborepo cache",
  generic_dist: "dist/",
  hugging_face_cache: "HuggingFace cache",
  ollama_models: "Ollama models",
  torch_cache: "PyTorch cache",
  triton_cache: "Triton cache",
};

const KIND_BADGE_CLASS = {
  node_modules: "badge-npm",
  python_venv: "badge-pnpm",
  python_pycache: "badge-pnpm",
  python_pytest_cache: "badge-pnpm",
  python_mypy_cache: "badge-pnpm",
  python_ruff_cache: "badge-pnpm",
  rust_target: "badge-yarn",
  java_maven_target: "badge-unknown",
  java_gradle_build: "badge-unknown",
  go_mod_cache: "badge-yarn",
  go_build_cache: "badge-yarn",
  php_vendor: "badge-unknown",
  dotnet_bin: "badge-unknown",
  dotnet_obj: "badge-unknown",
  next_cache: "badge-npm",
  turbo_cache: "badge-npm",
  generic_dist: "badge-unknown",
  hugging_face_cache: "badge-pnpm",
  ollama_models: "badge-yarn",
  torch_cache: "badge-npm",
  triton_cache: "badge-unknown",
};

const KIND_RISK_NOTES = {
  python_venv:
    "This is a Python virtual environment, not just a cache. If nothing has a requirements.txt/poetry.lock recording its exact packages, deleting it loses that environment for good — and if any running process, service, or script currently points at this venv's interpreter, it will break the moment this is gone.",
  hugging_face_cache:
    "This cache stores downloaded model weights (.safetensors, .bin). Deleting it requires re-downloading large multi-gigabyte models over the internet on next run.",
  ollama_models:
    "This directory contains local Ollama model blobs. Deleting it will require re-pulling models via 'ollama pull <model>'.",
  torch_cache:
    "This cache stores PyTorch model checkpoints and kernel cache. Deleting it requires re-downloading pretrained weights on next run.",
};

function badgeFor(entry) {
  const label = KIND_LABELS[entry.kind] || entry.kind;
  const cls = KIND_BADGE_CLASS[entry.kind] || "badge-unknown";
  let html = `<span class="badge ${cls}">${escapeHtml(label)}</span>`;
  if (entry.kind === "node_modules" && entry.package_manager) {
    html += ` <span class="group-meta">${escapeHtml(entry.package_manager)}</span>`;
  }
  return html;
}

function showToast(message, kind = "info") {
  const el = document.createElement("div");
  el.className = `toast ${kind}`;
  el.textContent = message;
  toastStack.appendChild(el);
  setTimeout(() => el.remove(), 4000);
}

// ── Scan tab ──────────────────────────────────────────────────────────────────

function allVisibleEntries() {
  if (viewMode === "flat") return filteredSorted(flatEntries);
  const out = [];
  for (const g of groups) {
    for (const e of g.entries) {
      if (matchesFilter(e)) out.push(e);
    }
  }
  return out;
}

function matchesFilter(entry) {
  if (excludedKinds.has(entry.kind)) return false;
  if (!filterText) return true;
  return entry.path.toLowerCase().includes(filterText.toLowerCase());
}

function filteredSorted(list) {
  const filtered = list.filter(matchesFilter);
  const dir = sortDir === "asc" ? 1 : -1;
  filtered.sort((a, b) => {
    if (sortKey === "size") return (a.size_bytes - b.size_bytes) * dir;
    if (sortKey === "pm") return (KIND_LABELS[a.kind] || a.kind).localeCompare(KIND_LABELS[b.kind] || b.kind) * dir;
    return a.path.localeCompare(b.path) * dir;
  });
  return filtered;
}

function keyFor(entry) { return entry.path; }

function renderRow(entry) {
  const row = document.createElement("tr");
  row.className = "entry-row";

  const checkCell = document.createElement("td");
  const checkbox = document.createElement("input");
  checkbox.type = "checkbox";
  checkbox.checked = selected.has(keyFor(entry));
  checkbox.addEventListener("change", () => {
    if (checkbox.checked) selected.add(keyFor(entry));
    else selected.delete(keyFor(entry));
    updateButtons();
  });
  checkCell.appendChild(checkbox);

  const sizeCell = document.createElement("td");
  sizeCell.className = "size-cell";
  sizeCell.textContent = humanSize(entry.size_bytes);

  const pmCell = document.createElement("td");
  pmCell.innerHTML = badgeFor(entry);

  const pathCell = document.createElement("td");
  pathCell.textContent = entry.path;
  pathCell.className = "path-cell";

  row.append(checkCell, sizeCell, pmCell, pathCell);
  return row;
}

function render() {
  resultsBody.innerHTML = "";
  let total = 0;
  let shownCount = 0;

  if (viewMode === "flat") {
    const list = filteredSorted(flatEntries);
    shownCount = list.length;
    list.forEach((e) => {
      total += e.size_bytes;
      resultsBody.appendChild(renderRow(e));
    });
  } else {
    groups.forEach((group, gIdx) => {
      const groupEntries = group.entries.filter(matchesFilter);
      if (groupEntries.length === 0) return;
      shownCount += groupEntries.length;
      const groupSize = groupEntries.reduce((s, e) => s + e.size_bytes, 0);
      total += groupSize;

      const headerRow = document.createElement("tr");
      headerRow.className = "group-header";
      const collapsed = collapsedGroups.has(gIdx);
      headerRow.innerHTML = `
        <td></td>
        <td colspan="3">
          <span class="group-toggle ${collapsed ? "collapsed" : ""}">▾</span>
          <span class="group-label">${escapeHtml(group.root)}</span>
          <span class="group-meta">${groupEntries.length} found · ${humanSize(groupSize)}</span>
        </td>
      `;
      headerRow.addEventListener("click", () => {
        if (collapsedGroups.has(gIdx)) collapsedGroups.delete(gIdx);
        else collapsedGroups.add(gIdx);
        render();
      });
      resultsBody.appendChild(headerRow);
      if (!collapsed) groupEntries.forEach((e) => resultsBody.appendChild(renderRow(e)));
    });
  }

  summary.textContent = `${shownCount} artifacts found · ${humanSize(total)} total`;
  emptyState.classList.toggle("hidden", shownCount !== 0);
  resultsTable.classList.toggle("hidden", shownCount === 0);

  const visible = allVisibleEntries();
  selectAll.checked = visible.length > 0 && visible.every((e) => selected.has(keyFor(e)));
  selectAll.indeterminate = !selectAll.checked && visible.some((e) => selected.has(keyFor(e)));

  updateButtons();
}

function updateButtons() {
  const hasSelection = selected.size > 0;
  trashBtn.disabled = !hasSelection;
  archiveBtn.disabled = !hasSelection;
  permanentBtn.disabled = !hasSelection;
}

async function doScan() {
  const root = rootInput.value.trim();
  if (!root) { showToast("Enter a folder to scan.", "error"); return; }
  spinner.classList.remove("hidden");
  scanBtn.disabled = true;
  scanBtn.textContent = "Scanning...";
  try {
    const [flat, grouped] = await Promise.all([
      invoke("scan_command", { root }),
      invoke("scan_grouped_command", { root }),
    ]);
    flatEntries = flat;
    groups = grouped;
    selected.clear();
    collapsedGroups.clear();
    render();
  } catch (e) {
    showToast(`Scan failed: ${e}`, "error");
  } finally {
    spinner.classList.add("hidden");
    scanBtn.disabled = false;
    scanBtn.textContent = "Scan";
  }
}

function selectedEntryObjects() {
  const all = viewMode === "flat" ? flatEntries : groups.flatMap((g) => g.entries);
  return all.filter((e) => selected.has(keyFor(e)));
}

async function performDelete(mode) {
  const targets = selectedEntryObjects();
  if (targets.length === 0) return;

  const paths = targets.map((e) => e.path);
  const sizes = targets.map((e) => e.size_bytes);

  try {
    const results = await invoke("delete_command", { paths, mode, sizes });
    const failed = results.filter((r) => r.error);
    const freed = results.filter((r) => !r.error).reduce((sum, r) => sum + r.freed_bytes, 0);
    const succeededPaths = new Set(results.filter((r) => !r.error).map((r) => r.path));

    flatEntries = flatEntries.filter((e) => !succeededPaths.has(e.path));
    groups = groups
      .map((g) => ({ ...g, entries: g.entries.filter((e) => !succeededPaths.has(e.path)) }))
      .filter((g) => g.entries.length > 0);
    selected.clear();
    render();

    if (failed.length) {
      showToast(`Freed ${humanSize(freed)}, ${failed.length} failed`, "error");
    } else {
      showToast(`Freed ${humanSize(freed)}`, "success");
    }
    freedTotal.textContent = `Last action freed ${humanSize(freed)}`;
  } catch (e) {
    showToast(`Delete failed: ${e}`, "error");
  }
}

function riskyNotesFor(targets) {
  const seen = new Set();
  const notes = [];
  for (const e of targets) {
    const note = KIND_RISK_NOTES[e.kind];
    if (note && !seen.has(e.kind)) {
      seen.add(e.kind);
      notes.push(`${KIND_LABELS[e.kind] || e.kind}: ${note}`);
    }
  }
  return notes;
}

let pendingMode = null;

function requestDelete(mode) {
  const targets = selectedEntryObjects();
  if (targets.length === 0) return;
  const risky = riskyNotesFor(targets);

  if (mode === "permanent") {
    pendingMode = "permanent";
    const count = targets.length;
    let body = `Permanently delete ${count} folder${count === 1 ? "" : "s"}? This cannot be undone.`;
    if (risky.length) body += `\n\n⚠ ${risky.join("\n\n⚠ ")}`;
    modalBody.textContent = body;
    modalConfirm.textContent = "Delete permanently";
    modalBackdrop.classList.remove("hidden");
    return;
  }

  if (risky.length) {
    pendingMode = mode;
    const label = mode === "trash" ? "move to trash" : "archive then delete";
    modalBody.textContent = `⚠ ${risky.join("\n\n⚠ ")}\n\nProceed to ${label}?`;
    modalConfirm.textContent = mode === "trash" ? "Move to trash" : "Archive then delete";
    modalBackdrop.classList.remove("hidden");
    return;
  }

  performDelete(mode);
}

function closeModal() { modalBackdrop.classList.add("hidden"); }

// ── Doctor tab ────────────────────────────────────────────────────────────────

async function runDoctor() {
  const root = rootInput.value.trim();
  if (!root) { showToast("Enter a folder in the header first.", "error"); return; }

  const doctorSpinner = document.getElementById("doctor-spinner");
  const doctorContent = document.getElementById("doctor-content");
  const doctorEmpty = document.getElementById("doctor-empty");

  doctorSpinner.classList.remove("hidden");
  doctorContent.classList.add("hidden");
  doctorEmpty.classList.add("hidden");

  try {
    const report = await invoke("doctor_command", { root });

    document.getElementById("doctor-total").innerHTML =
      `<strong>Total reclaimable:</strong> ${humanSize(report.total_bytes)} across ${report.total_count} artifact directories`;

    const body = document.getElementById("doctor-body");
    body.innerHTML = "";
    for (const eco of report.ecosystems) {
      const oldest = eco.oldest_days != null ? `${eco.oldest_days}d` : "—";
      const oldestPath = eco.oldest_path
        ? `<span title="${escapeHtml(eco.oldest_path)}">${escapeHtml(truncatePath(eco.oldest_path, 40))}</span>`
        : "—";
      const tr = document.createElement("tr");
      tr.innerHTML = `
        <td><span class="badge badge-unknown">${escapeHtml(eco.label)}</span></td>
        <td>${eco.count}</td>
        <td>${humanSize(eco.total_bytes)}</td>
        <td>${oldest}</td>
        <td class="path-cell">${oldestPath}</td>
      `;
      body.appendChild(tr);
    }

    // Top 5 biggest
    const allBiggest = report.ecosystems.flatMap((e) => e.biggest);
    allBiggest.sort((a, b) => b.size_bytes - a.size_bytes);
    const top5 = allBiggest.slice(0, 5);
    const biggestDiv = document.getElementById("doctor-biggest");
    if (top5.length > 0) {
      biggestDiv.innerHTML = `<h3>Top space consumers</h3>` +
        top5.map((b) => {
          const age = b.age_days != null ? `${b.age_days}d ago` : "unknown age";
          return `<div class="doctor-biggest-row">
            <span class="size-cell">${humanSize(b.size_bytes)}</span>
            <span class="group-meta">${age}</span>
            <span class="path-cell" title="${escapeHtml(b.path)}">${escapeHtml(truncatePath(b.path, 60))}</span>
          </div>`;
        }).join("");
    } else {
      biggestDiv.innerHTML = "";
    }

    doctorContent.classList.remove("hidden");
  } catch (e) {
    showToast(`Doctor failed: ${e}`, "error");
    doctorEmpty.classList.remove("hidden");
  } finally {
    doctorSpinner.classList.add("hidden");
  }
}

// ── Stats tab ─────────────────────────────────────────────────────────────────

async function loadStats() {
  const statsContent = document.getElementById("stats-content");
  const statsEmpty = document.getElementById("stats-empty");

  try {
    const data = await invoke("stats_command");

    if (data.total_deletions === 0) {
      statsContent.classList.add("hidden");
      statsEmpty.classList.remove("hidden");
      return;
    }

    document.getElementById("stats-total").innerHTML =
      `<strong>Total freed:</strong> ${humanSize(data.total_freed_bytes)} across ${data.total_deletions} deletion${data.total_deletions === 1 ? "" : "s"}`;

    const ecoBody = document.getElementById("stats-eco-body");
    ecoBody.innerHTML = "";
    for (const eco of data.by_ecosystem) {
      const lastDate = eco.last_deleted_at > 0
        ? formatRelative(eco.last_deleted_at)
        : "—";
      const tr = document.createElement("tr");
      tr.innerHTML = `
        <td>${escapeHtml(eco.kind)}</td>
        <td>${eco.count}</td>
        <td>${humanSize(eco.total_bytes)}</td>
        <td>${escapeHtml(lastDate)}</td>
      `;
      ecoBody.appendChild(tr);
    }

    // Bar chart — last 30 days
    const chartDiv = document.getElementById("stats-chart");
    const daily = data.daily || [];
    const cutoff = dateMinusDays(30);
    const recent = daily.filter((d) => d.date >= cutoff);

    if (recent.length > 0) {
      const maxBytes = Math.max(...recent.map((d) => d.bytes_freed));
      chartDiv.innerHTML = "<h3>Last 30 days</h3>" +
        recent.map((d) => {
          const barPct = maxBytes > 0 ? Math.max(4, (d.bytes_freed / maxBytes) * 100) : 4;
          return `<div class="chart-row">
            <span class="chart-date">${escapeHtml(d.date)}</span>
            <div class="chart-bar-wrap">
              <div class="chart-bar" style="width:${barPct}%"></div>
            </div>
            <span class="chart-label">${humanSize(d.bytes_freed)} (${d.count})</span>
          </div>`;
        }).join("");
    } else {
      chartDiv.innerHTML = "<p class='restore-empty'>No deletions in the last 30 days.</p>";
    }

    statsContent.classList.remove("hidden");
    statsEmpty.classList.add("hidden");
  } catch (e) {
    showToast(`Stats failed: ${e}`, "error");
  }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function truncatePath(path, max) {
  if (path.length <= max) return path;
  const half = Math.floor((max - 3) / 2);
  return path.slice(0, half) + "..." + path.slice(path.length - half);
}

function formatRelative(unixSecs) {
  const now = Math.floor(Date.now() / 1000);
  const diff = now - unixSecs;
  const days = Math.floor(diff / 86400);
  if (days === 0) return "today";
  if (days === 1) return "yesterday";
  return `${days} days ago`;
}

function dateMinusDays(n) {
  const d = new Date();
  d.setDate(d.getDate() - n);
  return d.toISOString().slice(0, 10);
}

// ── Event listeners ───────────────────────────────────────────────────────────

scanBtn.addEventListener("click", doScan);
rootInput.addEventListener("keydown", (e) => { if (e.key === "Enter") doScan(); });
searchInput.addEventListener("input", (e) => { filterText = e.target.value; render(); });

selectAll.addEventListener("change", () => {
  const visible = allVisibleEntries();
  if (selectAll.checked) visible.forEach((e) => selected.add(keyFor(e)));
  else visible.forEach((e) => selected.delete(keyFor(e)));
  render();
});

document.querySelectorAll("th.sortable").forEach((th) => {
  th.addEventListener("click", () => {
    const key = th.dataset.sort;
    if (sortKey === key) sortDir = sortDir === "asc" ? "desc" : "asc";
    else { sortKey = key; sortDir = "desc"; }
    document.querySelectorAll(".sort-arrow").forEach((s) => (s.textContent = ""));
    th.querySelector(".sort-arrow").textContent = sortDir === "asc" ? "↑" : "↓";
    render();
  });
});

viewFlatBtn.addEventListener("click", () => {
  viewMode = "flat";
  viewFlatBtn.classList.add("active");
  viewGroupedBtn.classList.remove("active");
  render();
});
viewGroupedBtn.addEventListener("click", () => {
  viewMode = "grouped";
  viewGroupedBtn.classList.add("active");
  viewFlatBtn.classList.remove("active");
  render();
});

trashBtn.addEventListener("click", () => requestDelete("trash"));
archiveBtn.addEventListener("click", () => requestDelete("archive"));
permanentBtn.addEventListener("click", () => requestDelete("permanent"));
modalCancel.addEventListener("click", closeModal);
modalConfirm.addEventListener("click", () => {
  const mode = pendingMode;
  closeModal();
  if (mode) performDelete(mode);
});
modalBackdrop.addEventListener("click", (e) => { if (e.target === modalBackdrop) closeModal(); });

document.getElementById("doctor-btn").addEventListener("click", runDoctor);
document.getElementById("stats-btn").addEventListener("click", loadStats);

const restoreBtn = document.getElementById("restore-btn");
const restorePanel = document.getElementById("restore-panel");
const restoreClose = document.getElementById("restore-close");
const restoreList = document.getElementById("restore-list");

async function openRestorePanel() {
  restorePanel.classList.remove("hidden");
  restoreList.innerHTML = '<p class="restore-empty">Searching for archives...</p>';
  const root = rootInput.value.trim() || "/";
  try {
    const archives = await invoke("find_archives_command", { root });
    if (archives.length === 0) {
      restoreList.innerHTML = '<p class="restore-empty">No backup archives found. Create one using "Archive then Delete".</p>';
      return;
    }
    restoreList.innerHTML = "";
    for (const arc of archives) {
      const item = document.createElement("div");
      item.className = "restore-item";
      item.innerHTML = `
        <span class="restore-item-size">${humanSize(arc.size_bytes)}</span>
        <span class="restore-item-path" title="${escapeHtml(arc.path)}">${escapeHtml(arc.path)}</span>
        <button class="restore-item-btn">Restore</button>
      `;
      item.querySelector(".restore-item-btn").addEventListener("click", async () => {
        try {
          const restored = await invoke("restore_command", { archivePath: arc.path });
          showToast(`Restored to ${restored}`, "success");
          openRestorePanel();
        } catch (e) {
          showToast(`Restore failed: ${e}`, "error");
        }
      });
      restoreList.appendChild(item);
    }
  } catch (e) {
    restoreList.innerHTML = `<p class="restore-empty">Error: ${escapeHtml(e)}</p>`;
  }
}

restoreBtn.addEventListener("click", openRestorePanel);
restoreClose.addEventListener("click", () => restorePanel.classList.add("hidden"));

invoke("home_dir_command").then((home) => { rootInput.value = home; });
renderChips();
