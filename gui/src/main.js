const { invoke } = window.__TAURI__.core;

let flatEntries = [];
let groups = [];
let viewMode = "flat";
let sortKey = "size";
let sortDir = "desc";
let filterText = "";
const selected = new Set();
const collapsedGroups = new Set();

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

function badgeFor(pm) {
  const cls = { Npm: "badge-npm", Yarn: "badge-yarn", Pnpm: "badge-pnpm" }[pm] || "badge-unknown";
  return `<span class="badge ${cls}">${pm}</span>`;
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
  if (!filterText) return true;
  return entry.path.toLowerCase().includes(filterText.toLowerCase());
}

function filteredSorted(list) {
  const filtered = list.filter(matchesFilter);
  const dir = sortDir === "asc" ? 1 : -1;
  filtered.sort((a, b) => {
    if (sortKey === "size") return (a.size_bytes - b.size_bytes) * dir;
    if (sortKey === "pm") return a.package_manager.localeCompare(b.package_manager) * dir;
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
  pmCell.innerHTML = badgeFor(entry.package_manager);

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

  summary.textContent = `${shownCount} node_modules found · ${humanSize(total)} total`;
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

function openPermanentConfirm() {
  const count = selected.size;
  modalBody.textContent = `Permanently delete ${count} folder${count === 1 ? "" : "s"}? This cannot be undone.`;
  modalBackdrop.classList.remove("hidden");
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

trashBtn.addEventListener("click", () => performDelete("trash"));
archiveBtn.addEventListener("click", () => performDelete("archive"));
permanentBtn.addEventListener("click", openPermanentConfirm);
modalCancel.addEventListener("click", closeModal);
modalConfirm.addEventListener("click", () => {
  closeModal();
  performDelete("permanent");
});
modalBackdrop.addEventListener("click", (e) => {
  if (e.target === modalBackdrop) closeModal();
});

// Prefill with the user's home directory on launch.
invoke("home_dir_command").then((home) => {
  rootInput.value = home;
});
