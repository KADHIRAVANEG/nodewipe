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
};

const KIND_RISK_NOTES = {
  python_venv:
    "This is a Python virtual environment, not just a cache. If nothing has a requirements.txt/poetry.lock recording its exact packages, deleting it loses that environment for good — and if any running process, service, or script currently points at this venv's interpreter, it will break the moment this is gone.",
};

function badgeFor(entry) {
  const label = KIND_LABELS[entry.kind] || entry.kind;
  const cls = KIND_BADGE_CLASS[entry.kind] || "badge-unknown";
  let html = `<span class="badge ${cls}">${label}</span>`;
  if (entry.kind === "node_modules" && entry.package_manager) {
    html += ` <span class="group-meta">${entry.package_manager}</span>`;
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

function allVisibleEntries() {
  // Flattened view of whatever is currently on screen, in display order,
  // used for select-all and bulk actions regardless of flat/grouped mode.
  if (viewMode === "flat") {
    return filteredSorted(flatEntries);
  }
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

function keyFor(entry) {
  return entry.path;
}

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
          <span class="group-label">${group.root}</span>
          <span class="group-meta">${groupEntries.length} found · ${humanSize(groupSize)}</span>
        </td>
      `;
      headerRow.addEventListener("click", () => {
        if (collapsedGroups.has(gIdx)) collapsedGroups.delete(gIdx);
        else collapsedGroups.add(gIdx);
        render();
      });
      resultsBody.appendChild(headerRow);

      if (!collapsed) {
        groupEntries.forEach((e) => resultsBody.appendChild(renderRow(e)));
      }
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
  if (!root) {
    showToast("Enter a folder to scan.", "error");
    return;
  }
  spinner.classList.remove("hidden");
  scanBtn.disabled = true;
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

/// Gate before any delete button acts. Permanent always confirms via the
/// modal. Trash/Archive normally proceed immediately (Trash is recoverable,
/// Archive keeps a backup) — but if the selection includes a risky kind
/// (e.g. a Python venv), they route through the same modal first, since
/// "recoverable in the OS trash" doesn't help a process that breaks the
/// instant the venv disappears from disk.
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

function closeModal() {
  modalBackdrop.classList.add("hidden");
}

scanBtn.addEventListener("click", doScan);
rootInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") doScan();
});
searchInput.addEventListener("input", (e) => {
  filterText = e.target.value;
  render();
});

selectAll.addEventListener("change", () => {
  const visible = allVisibleEntries();
  if (selectAll.checked) {
    visible.forEach((e) => selected.add(keyFor(e)));
  } else {
    visible.forEach((e) => selected.delete(keyFor(e)));
  }
  render();
});

document.querySelectorAll("th.sortable").forEach((th) => {
  th.addEventListener("click", () => {
    const key = th.dataset.sort;
    if (sortKey === key) {
      sortDir = sortDir === "asc" ? "desc" : "asc";
    } else {
      sortKey = key;
      sortDir = "desc";
    }
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
modalBackdrop.addEventListener("click", (e) => {
  if (e.target === modalBackdrop) closeModal();
});

// Prefill with the user's home directory on launch.
invoke("home_dir_command").then((home) => {
  rootInput.value = home;
});

// Build the type-filter chip row now that KIND_LABELS exists.
renderChips();
