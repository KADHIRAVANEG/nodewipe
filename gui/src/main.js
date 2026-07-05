const { invoke } = window.__TAURI__.core;

let entries = [];
const selected = new Set();

const rootInput = document.getElementById("root-input");
const scanBtn = document.getElementById("scan-btn");
const resultsBody = document.getElementById("results-body");
const summary = document.getElementById("summary");
const status = document.getElementById("status");
const trashBtn = document.getElementById("trash-btn");
const archiveBtn = document.getElementById("archive-btn");
const permanentBtn = document.getElementById("permanent-btn");

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

function updateButtons() {
  const hasSelection = selected.size > 0;
  trashBtn.disabled = !hasSelection;
  archiveBtn.disabled = !hasSelection;
  permanentBtn.disabled = !hasSelection;
}

function render() {
  resultsBody.innerHTML = "";
  let total = 0;

  entries.forEach((entry, idx) => {
    total += entry.size_bytes;
    const row = document.createElement("tr");

    const checkCell = document.createElement("td");
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = selected.has(idx);
    checkbox.addEventListener("change", () => {
      if (checkbox.checked) {
        selected.add(idx);
      } else {
        selected.delete(idx);
      }
      updateButtons();
    });
    checkCell.appendChild(checkbox);

    const sizeCell = document.createElement("td");
    sizeCell.textContent = humanSize(entry.size_bytes);

    const pmCell = document.createElement("td");
    pmCell.textContent = entry.package_manager;

    const pathCell = document.createElement("td");
    pathCell.textContent = entry.path;
    pathCell.className = "path-cell";

    row.append(checkCell, sizeCell, pmCell, pathCell);
    resultsBody.appendChild(row);
  });

  summary.textContent = `${entries.length} node_modules found · ${humanSize(total)} total`;
  updateButtons();
}

async function doScan() {
  const root = rootInput.value.trim();
  if (!root) {
    status.textContent = "Enter a folder to scan.";
    return;
  }
  status.textContent = "Scanning...";
  scanBtn.disabled = true;
  try {
    entries = await invoke("scan_command", { root });
    selected.clear();
    render();
    status.textContent = "";
  } catch (e) {
    status.textContent = `Error: ${e}`;
  } finally {
    scanBtn.disabled = false;
  }
}

async function doDelete(mode) {
  if (selected.size === 0) return;

  if (mode === "permanent") {
    const ok = window.confirm(`Permanently delete ${selected.size} folder(s)? This cannot be undone.`);
    if (!ok) return;
  }

  const idxList = Array.from(selected);
  const paths = idxList.map((i) => entries[i].path);
  const sizes = idxList.map((i) => entries[i].size_bytes);

  status.textContent = "Deleting...";
  try {
    const results = await invoke("delete_command", { paths, mode, sizes });
    const failed = results.filter((r) => r.error);
    const freed = results.filter((r) => !r.error).reduce((sum, r) => sum + r.freed_bytes, 0);

    // Remove successfully deleted entries from the table (highest index first).
    const succeededPaths = new Set(results.filter((r) => !r.error).map((r) => r.path));
    entries = entries.filter((e) => !succeededPaths.has(e.path));
    selected.clear();
    render();

    status.textContent = failed.length
      ? `Freed ${humanSize(freed)}, ${failed.length} failed`
      : `Freed ${humanSize(freed)}`;
  } catch (e) {
    status.textContent = `Error: ${e}`;
  }
}

scanBtn.addEventListener("click", doScan);
rootInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") doScan();
});
trashBtn.addEventListener("click", () => doDelete("trash"));
archiveBtn.addEventListener("click", () => doDelete("archive"));
permanentBtn.addEventListener("click", () => doDelete("permanent"));

// Prefill with the user's home directory on launch.
invoke("home_dir_command").then((home) => {
  rootInput.value = home;
});
