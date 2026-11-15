document.addEventListener('DOMContentLoaded', () => {
    // ==========================================
    // 1. INSTALL SWITCHER TABS & CLIPBOARD COPY
    // ==========================================
    const installData = {
        npm: {
            cmd: "npx @joker53/nodewipe",
            caption: "Runs instantly without permanent install via native wrapper."
        },
        brew: {
            cmd: "brew install kadhiravaneg/nodewipe/nodewipe",
            caption: "Installs native CLI, man page, and shell completions for macOS & Linux."
        },
        aur: {
            cmd: "yay -S nodewipe",
            caption: "Native Arch Linux package from the Arch User Repository."
        },
        cargo: {
            cmd: "cargo install nodewipe",
            caption: "Compiles from source directly with your local Rust toolchain."
        },
        winget: {
            cmd: "winget install nodewipe",
            caption: "Official Microsoft Windows Package Manager portable installer."
        },
        scoop: {
            cmd: "scoop install nodewipe",
            caption: "Fast, community-driven Windows package manager manifest."
        },
        docker: {
            cmd: 'docker run --rm -it -v "$PWD":/scan ghcr.io/joker-53/nodewipe',
            caption: "Run isolated scans anywhere without installing anything on your host."
        },
        curl: {
            cmd: "curl -fsSL https://raw.githubusercontent.com/KADHIRAVANEG/nodewipe/main/install.sh | bash",
            caption: "Universal self-contained binary installer with auto-architecture detection."
        }
    };

    const installTabs = document.querySelectorAll('.install-tab');
    const installCmdEl = document.getElementById('installCmd');
    const installCaptionEl = document.getElementById('installCaption');
    const copyBtn = document.getElementById('copyBtn');

    installTabs.forEach(tab => {
        tab.addEventListener('click', () => {
            installTabs.forEach(t => t.classList.remove('active'));
            tab.classList.add('active');
            const pkg = tab.dataset.pkg;
            if (installData[pkg]) {
                installCmdEl.textContent = installData[pkg].cmd;
                installCaptionEl.textContent = installData[pkg].caption;
            }
        });
    });

    if (copyBtn) {
        copyBtn.addEventListener('click', () => {
            const textToCopy = installCmdEl.textContent.trim();
            navigator.clipboard.writeText(textToCopy).then(() => {
                copyBtn.classList.add('copied');
                const copyTextEl = copyBtn.querySelector('.copy-text');
                if (copyTextEl) copyTextEl.textContent = 'Copied!';
                setTimeout(() => {
                    copyBtn.classList.remove('copied');
                    if (copyTextEl) copyTextEl.textContent = 'Copy';
                }, 2200);
            }).catch(err => {
                console.error('Failed to copy command: ', err);
            });
        });
    }

    // ==========================================
    // 2. INTERACTIVE TERMINAL TUI SIMULATOR
    // ==========================================
    const initialSimItems = [
        { id: 1, path: '~/code/storefront-app/node_modules', bytes: 1954283520, size: '1.82 GB', type: 'Node / npm', git: 'git-clean', selected: true },
        { id: 2, path: '~/dev/rusty-engine/target', bytes: 7999324160, size: '7.45 GB', type: 'Rust / Cargo', git: 'git-dirty', selected: false },
        { id: 3, path: '~/ai/llama3-finetune/.venv', bytes: 3435973836, size: '3.20 GB', type: 'Python / uv', git: 'git-clean', selected: true },
        { id: 4, path: '~/ios/swift-client/DerivedData', bytes: 12669984768, size: '11.80 GB', type: 'Apple / Xcode', git: 'git-clean', selected: false },
        { id: 5, path: '~/services/go-worker/vendor', bytes: 461373440, size: '440 MB', type: 'Go Module', git: 'git-clean', selected: false },
        { id: 6, path: '~/models/huggingface/hub', bytes: 15247133696, size: '14.20 GB', type: 'AI/ML Cache', git: 'clean', selected: false }
    ];

    let simItems = JSON.parse(JSON.stringify(initialSimItems));
    let simCursor = 0;

    const tuiViewport = document.getElementById('tuiViewport');
    const simSelectedSizeEl = document.getElementById('simSelectedSize');
    const simSelectedCountEl = document.getElementById('simSelectedCount');
    const simSuccessModal = document.getElementById('simSuccessModal');
    const simDryrunModal = document.getElementById('simDryrunModal');
    const simSuccessMsg = document.getElementById('simSuccessMsg');
    const simPreviewList = document.getElementById('simPreviewList');

    function formatBytes(bytes) {
        if (bytes === 0) return '0 B';
        const k = 1024;
        const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
        const i = Math.floor(Math.log(bytes) / Math.log(k));
        return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    }

    function updateSimStats() {
        const selected = simItems.filter(item => item.selected);
        const totalSelectedBytes = selected.reduce((acc, item) => acc + item.bytes, 0);
        simSelectedCountEl.textContent = selected.length;
        simSelectedSizeEl.textContent = formatBytes(totalSelectedBytes);
    }

    function renderTui() {
        tuiViewport.innerHTML = '';
        if (simItems.length === 0) {
            tuiViewport.innerHTML = '<div style="padding: 2rem; text-align: center; color: var(--accent-emerald);">✨ Workspace is completely clean! Zero disposable artifacts remaining.</div>';
            updateSimStats();
            return;
        }

        simItems.forEach((item, index) => {
            const row = document.createElement('div');
            row.className = `tui-row ${index === simCursor ? 'active' : ''} ${item.selected ? 'selected' : ''}`;
            row.innerHTML = `
                <span class="tui-check">${item.selected ? '[✓]' : '[ ]'}</span>
                <span class="tui-path">${item.path}</span>
                <span class="tui-size">${item.size}</span>
                <span class="tui-badge ${item.git}">${item.type}</span>
            `;
            row.addEventListener('click', () => {
                simCursor = index;
                item.selected = !item.selected;
                renderTui();
            });
            tuiViewport.appendChild(row);
        });

        updateSimStats();
    }

    // Keyboard navigation within TUI
    document.addEventListener('keydown', (e) => {
        // Only handle if simulator is in view or user interacted
        const rect = tuiViewport.getBoundingClientRect();
        if (rect.top > window.innerHeight || rect.bottom < 0) return;

        if (e.key === 'ArrowDown') {
            e.preventDefault();
            if (simCursor < simItems.length - 1) {
                simCursor++;
                renderTui();
            }
        } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            if (simCursor > 0) {
                simCursor--;
                renderTui();
            }
        } else if (e.key === ' ' || e.code === 'Space') {
            if (document.activeElement.tagName !== 'INPUT' && document.activeElement.tagName !== 'TEXTAREA') {
                e.preventDefault();
                if (simItems[simCursor]) {
                    simItems[simCursor].selected = !simItems[simCursor].selected;
                    renderTui();
                }
            }
        } else if (e.key === 'p' || e.key === 'P') {
            if (document.activeElement.tagName !== 'INPUT') {
                e.preventDefault();
                openDryrunModal();
            }
        } else if (e.key === 'd' || e.key === 'D') {
            if (document.activeElement.tagName !== 'INPUT') {
                e.preventDefault();
                executeSimDeletion();
            }
        } else if (e.key === 's' || e.key === 'S') {
            if (document.activeElement.tagName !== 'INPUT') {
                window.open('https://github.com/KADHIRAVANEG/nodewipe', '_blank');
            }
        }
    });

    // Simulator Buttons
    const simBtnSelect = document.getElementById('simBtnSelect');
    const simBtnPreview = document.getElementById('simBtnPreview');
    const simBtnDelete = document.getElementById('simBtnDelete');
    const simResetBtn = document.getElementById('simResetBtn');

    if (simBtnSelect) {
        simBtnSelect.addEventListener('click', () => {
            if (simItems[simCursor]) {
                simItems[simCursor].selected = !simItems[simCursor].selected;
                renderTui();
            }
        });
    }

    function openDryrunModal() {
        const selected = simItems.filter(item => item.selected);
        if (selected.length === 0) {
            alert('Please select at least one item first (press Space or click row).');
            return;
        }

        simPreviewList.innerHTML = '';
        selected.forEach(item => {
            const div = document.createElement('div');
            div.className = 'preview-item';
            div.innerHTML = `
                <span>${item.path}</span>
                <strong style="color: var(--accent-amber);">${item.size}</strong>
            `;
            simPreviewList.appendChild(div);
        });
        simDryrunModal.classList.remove('hidden');
    }

    function executeSimDeletion() {
        const selected = simItems.filter(item => item.selected);
        if (selected.length === 0) {
            alert('Please select at least one item first (press Space or click row).');
            return;
        }

        const totalBytes = selected.reduce((acc, item) => acc + item.bytes, 0);
        const freedFormatted = formatBytes(totalBytes);

        simSuccessMsg.textContent = `Successfully wiped ${freedFormatted} across ${selected.length} projects safely to OS Trash!`;
        simItems = simItems.filter(item => !item.selected);
        if (simCursor >= simItems.length) {
            simCursor = Math.max(0, simItems.length - 1);
        }
        renderTui();
        simDryrunModal.classList.add('hidden');
        simSuccessModal.classList.remove('hidden');
    }

    if (simBtnPreview) simBtnPreview.addEventListener('click', openDryrunModal);
    if (simBtnDelete) simBtnDelete.addEventListener('click', executeSimDeletion);

    if (simResetBtn) {
        simResetBtn.addEventListener('click', () => {
            simItems = JSON.parse(JSON.stringify(initialSimItems));
            simCursor = 0;
            renderTui();
            simSuccessModal.classList.add('hidden');
            simDryrunModal.classList.add('hidden');
        });
    }

    document.getElementById('simModalClose')?.addEventListener('click', () => {
        simSuccessModal.classList.add('hidden');
    });

    document.getElementById('simDryrunClose')?.addEventListener('click', () => {
        simDryrunModal.classList.add('hidden');
    });

    document.getElementById('simDryrunDismiss')?.addEventListener('click', () => {
        simDryrunModal.classList.add('hidden');
    });

    document.getElementById('simDryrunExecute')?.addEventListener('click', executeSimDeletion);

    // Initial TUI render
    renderTui();

    // ==========================================
    // 3. ECOSYSTEM MATRIX & INSTANT SEARCH
    // ==========================================
    const ecosystems = [
        { name: 'Node.js & npm / pnpm / bun', cat: 'web', target: 'node_modules', marker: 'package.json', icon: '📦' },
        { name: 'Rust Cargo', cat: 'systems', target: 'target/', marker: 'Cargo.toml', icon: '🦀' },
        { name: 'Python Virtualenv', cat: 'python', target: '.venv, venv, env', marker: 'pyproject.toml, requirements.txt', icon: '🐍' },
        { name: 'Python Bytecode', cat: 'python', target: '__pycache__, .pytest_cache', marker: '*.py, setup.py', icon: '⚡' },
        { name: 'Apple Xcode & Swift', cat: 'mobile', target: 'DerivedData, .build', marker: '*.xcodeproj, Package.swift', icon: '🍏' },
        { name: 'iOS CocoaPods', cat: 'mobile', target: 'Pods/', marker: 'Podfile', icon: '📱' },
        { name: 'Android & Gradle', cat: 'mobile', target: '.gradle, build/', marker: 'build.gradle, gradlew', icon: '🤖' },
        { name: 'C/C++ CMake & Ninja', cat: 'systems', target: 'build/, cmake-build-*', marker: 'CMakeLists.txt', icon: '⚙️' },
        { name: 'Zig Build Artifacts', cat: 'systems', target: 'zig-out/, zig-cache/', marker: 'build.zig', icon: '⚡' },
        { name: 'Elixir Mix', cat: 'systems', target: '_build/, deps/', marker: 'mix.exs', icon: '💧' },
        { name: 'Next.js App Router', cat: 'web', target: '.next/', marker: 'next.config.js, package.json', icon: '▲' },
        { name: 'Nuxt 3 / Vue', cat: 'web', target: '.nuxt/, .output/', marker: 'nuxt.config.ts', icon: '💚' },
        { name: 'SvelteKit', cat: 'web', target: '.svelte-kit/', marker: 'svelte.config.js', icon: '🔥' },
        { name: 'Astro Framework', cat: 'web', target: '.astro/, dist/', marker: 'astro.config.mjs', icon: '🚀' },
        { name: 'Vite & Docusaurus', cat: 'web', target: 'dist/, .docusaurus/', marker: 'vite.config.ts', icon: '⚡' },
        { name: 'E2E Testing (Playwright/Cypress)', cat: 'web', target: 'test-results/, cypress/videos', marker: 'playwright.config.ts', icon: '🎭' },
        { name: 'AI/ML Hugging Face Hub', cat: 'python', target: 'models--*, hub/', marker: 'model weights, config.json', icon: '🤗' },
        { name: 'AI/ML Ollama Local Models', cat: 'python', target: 'manifests, blobs/', marker: 'Modelfile', icon: '🦙' },
        { name: 'PyTorch & Triton Kernels', cat: 'python', target: '.triton, torch_cache', marker: 'torch modules', icon: '🔥' },
        { name: 'Java Maven', cat: 'backend', target: 'target/', marker: 'pom.xml', icon: '☕' },
        { name: 'Go Build & Vendor', cat: 'backend', target: 'vendor/', marker: 'go.mod', icon: '🐹' },
        { name: '.NET / C# Caches', cat: 'backend', target: 'bin/, obj/', marker: '*.csproj, *.sln', icon: '🟣' },
        { name: 'PHP Composer', cat: 'backend', target: 'vendor/', marker: 'composer.json', icon: '🐘' },
        { name: 'Docker Overlay Layers', cat: 'systems', target: 'overlay2/, buildx/', marker: 'Dockerfile', icon: '🐳' }
    ];

    const ecoGrid = document.getElementById('ecoGrid');
    const ecoTabs = document.querySelectorAll('.eco-tab');
    const ecoSearch = document.getElementById('ecoSearch');

    let activeEcoCategory = 'all';
    let ecoSearchQuery = '';

    function renderEcosystems() {
        ecoGrid.innerHTML = '';
        const filtered = ecosystems.filter(eco => {
            const matchesCat = activeEcoCategory === 'all' || eco.cat === activeEcoCategory;
            const matchesSearch = eco.name.toLowerCase().includes(ecoSearchQuery) ||
                                  eco.target.toLowerCase().includes(ecoSearchQuery) ||
                                  eco.marker.toLowerCase().includes(ecoSearchQuery);
            return matchesCat && matchesSearch;
        });

        if (filtered.length === 0) {
            ecoGrid.innerHTML = '<div style="grid-column: 1/-1; padding: 2rem; text-align: center; color: var(--text-muted);">No matching ecosystems found for your search.</div>';
            return;
        }

        filtered.forEach(eco => {
            const card = document.createElement('div');
            card.className = 'eco-card';
            card.innerHTML = `
                <div class="eco-card-header">
                    <div class="eco-icon-title">
                        <span class="eco-icon">${eco.icon}</span>
                        <span class="eco-title">${eco.name}</span>
                    </div>
                    <span class="eco-category-tag">${eco.cat}</span>
                </div>
                <div class="eco-target-box">Target: ${eco.target}</div>
                <div class="eco-marker">Safety marker: <code>${eco.marker}</code></div>
            `;
            ecoGrid.appendChild(card);
        });
    }

    ecoTabs.forEach(tab => {
        tab.addEventListener('click', () => {
            ecoTabs.forEach(t => t.classList.remove('active'));
            tab.classList.add('active');
            activeEcoCategory = tab.dataset.category;
            renderEcosystems();
        });
    });

    if (ecoSearch) {
        ecoSearch.addEventListener('input', (e) => {
            ecoSearchQuery = e.target.value.toLowerCase().trim();
            renderEcosystems();
        });
    }

    renderEcosystems();

    // ==========================================
    // 4. DOCUMENTATION TAB SWITCHER
    // ==========================================
    const docsData = {
        tui: `
            <h3>Interactive Terminal UI</h3>
            <p>Launch nodewipe in full interactive mode with keyboard navigation, live filtering, and zero-risk dry-run preview.</p>
            <div class="code-block">
                <span class="cmd">nodewipe</span> <span class="flg">[PATH]</span> <span class="cmt"># Defaults to current directory</span>
            </div>
            <table class="docs-table">
                <tr><th>Key</th><th>Action</th></tr>
                <tr><td><code>↑ / ↓</code> or <code>j / k</code></td><td>Navigate up and down the artifact list</td></tr>
                <tr><td><code>Space</code></td><td>Toggle selection of targeted directory</td></tr>
                <tr><td><code>P</code></td><td>Open <strong>Dry-Run Preview</strong> to inspect deletions safely</td></tr>
                <tr><td><code>d</code></td><td>Wipe selected directories (moves to OS Trash by default)</td></tr>
                <tr><td><code>b</code></td><td>Create atomic Copy-on-Write / <code>.tar.gz</code> archive backup before delete</td></tr>
                <tr><td><code>/</code></td><td>Instant fuzzy search and path filter</td></tr>
                <tr><td><code>s</code> or <code>S</code></td><td>Open nodewipe GitHub page in browser to star repository</td></tr>
                <tr><td><code>q</code></td><td>Quit interactive session</td></tr>
            </table>
        `,
        scan: `
            <h3>Headless Scan Command</h3>
            <p>Scan directories non-interactively. Perfect for CI/CD runners, automation scripts, and disk usage analysis.</p>
            <div class="code-block">
                <span class="cmd">nodewipe scan</span> <span class="flg">--json --min-size 100M ~/Projects</span>
            </div>
            <table class="docs-table">
                <tr><th>Option</th><th>Description</th></tr>
                <tr><td><code>--json</code></td><td>Output results as machine-readable JSON array</td></tr>
                <tr><td><code>--min-size &lt;SIZE&gt;</code></td><td>Filter artifacts smaller than threshold (e.g. <code>50M</code>, <code>1G</code>)</td></tr>
                <tr><td><code>--types &lt;LIST&gt;</code></td><td>Only scan specific types: <code>node,rust,python,xcode,gradle</code></td></tr>
                <tr><td><code>--exclude-types &lt;LIST&gt;</code></td><td>Exclude specific types from analysis</td></tr>
                <tr><td><code>--depth &lt;N&gt;</code></td><td>Limit recursive scan depth</td></tr>
            </table>
        `,
        delete: `
            <h3>Autonomous Delete Command</h3>
            <p>Delete disposable artifacts non-interactively with strict safety controls.</p>
            <div class="code-block">
                <span class="cmd">nodewipe delete</span> <span class="flg">--archive --git-clean ~/Projects</span>
            </div>
            <table class="docs-table">
                <tr><th>Option</th><th>Description</th></tr>
                <tr><td><code>--trash</code></td><td>Move files to OS Trash (default behavior)</td></tr>
                <tr><td><code>--archive</code></td><td>Snapshot to CoW reflink or <code>.tar.gz</code> archive before deletion</td></tr>
                <tr><td><code>--permanent</code></td><td>Direct permanent unlinking (skips trash)</td></tr>
                <tr><td><code>--git-clean</code></td><td>Refuse to delete projects with uncommitted Git modifications</td></tr>
                <tr><td><code>--yes / -y</code></td><td>Bypass confirmation prompts for headless automation</td></tr>
            </table>
        `,
        doctor: `
            <h3>System Doctor Audit</h3>
            <p>Perform a system-wide disk-hog audit. Discovers your oldest untouched build directories and surfaces high-impact quick wins.</p>
            <div class="code-block">
                <span class="cmd">nodewipe doctor</span> <span class="flg">~</span>
            </div>
            <table class="docs-table">
                <tr><th>Feature</th><th>Description</th></tr>
                <tr><td><strong>Stale Cache Ranking</strong></td><td>Surfaces build directories untouched for months</td></tr>
                <tr><td><strong>Git Staleness Correlation</strong></td><td>Checks whether the project repository is archived or dormant</td></tr>
                <tr><td><strong>Quick Wins</strong></td><td>Lists top 5 largest items that yield maximum reclaimed storage</td></tr>
            </table>
        `,
        dedupe: `
            <h3>Hardlink Deduplication</h3>
            <p>Save gigabytes across monorepos and local clones by hardlinking identical files across <code>node_modules</code> and build trees.</p>
            <div class="code-block">
                <span class="cmd">nodewipe dedupe</span> <span class="flg">--dry-run ~/Projects</span>
            </div>
            <p>Computes BLAKE3 hashes of duplicate package files and replaces redundant copies with atomic filesystem hardlinks, preserving zero behavior change.</p>
        `,
        rebuild: `
            <h3>One-Touch Project Rebuilder</h3>
            <p>Restores previously wiped projects automatically using the appropriate package manager.</p>
            <div class="code-block">
                <span class="cmd">nodewipe rebuild</span> <span class="flg">~/Projects/my-app</span>
            </div>
            <p>Automatically detects <code>pnpm-lock.yaml</code> (runs <code>pnpm install</code>), <code>yarn.lock</code> (runs <code>yarn</code>), <code>package-lock.json</code> (runs <code>npm ci</code>), <code>Cargo.lock</code> (runs <code>cargo build</code>), or <code>uv.lock</code>.</p>
        `,
        safety: `
            <h3>Safety Architecture & CoW Reflex</h3>
            <p>Safety is the primary engineering invariant of nodewipe.</p>
            <table class="docs-table">
                <tr><th>Layer</th><th>Safeguard</th></tr>
                <tr><td><strong>Marker Check</strong></td><td>Never deletes ambiguous folders (like <code>dist</code> or <code>build</code>) without a validating marker.</td></tr>
                <tr><td><strong>OS Trash</strong></td><td>Uses <code>trash-rs</code> for FreeDesktop Trash (Linux), Finder Trash (macOS), and Recycle Bin (Windows).</td></tr>
                <tr><td><strong>CoW Snapshot</strong></td><td>Instant zero-space reflink clones on Btrfs, APFS, and XFS.</td></tr>
                <tr><td><strong>Safe Unpack</strong></td><td>Archives prevent path traversal (<code>../</code>) exploits during restoration.</td></tr>
                <tr><td><strong>.nodewipeignore</strong></td><td>Respects custom root and nested ignore globs.</td></tr>
            </table>
        `,
        ci: `
            <h3>Official GitHub Action</h3>
            <p>Clean CI runner caches and reclaim gigabytes before heavy Docker builds or integration runs.</p>
            <div class="code-block">
<span class="cmt"># .github/workflows/cleanup.yml</span>
- <span class="cmd">name</span>: Reclaim Disk Space with nodewipe
  <span class="cmd">uses</span>: <span class="flg">KADHIRAVANEG/nodewipe/.github/actions/nodewipe@main</span>
  <span class="cmd">with</span>:
    <span class="flg">path</span>: '.'
    <span class="flg">types</span>: 'node,rust,python'
    <span class="flg">min-size</span>: '50M'
            </div>
        `
    };

    const docsNavItems = document.querySelectorAll('.docs-nav-item');
    const docsContent = document.getElementById('docsContent');

    function renderDoc(docKey) {
        if (docsData[docKey]) {
            docsContent.innerHTML = docsData[docKey];
        }
    }

    docsNavItems.forEach(item => {
        item.addEventListener('click', () => {
            docsNavItems.forEach(n => n.classList.remove('active'));
            item.classList.add('active');
            renderDoc(item.dataset.doc);
        });
    });

    // Initial Doc Render
    renderDoc('tui');

    // ==========================================
    // 5. FAQ ACCORDION
    // ==========================================
    const faqItems = document.querySelectorAll('.faq-item');
    faqItems.forEach(item => {
        const questionBtn = item.querySelector('.faq-question');
        questionBtn.addEventListener('click', () => {
            const isOpen = item.classList.contains('open');
            faqItems.forEach(i => i.classList.remove('open'));
            if (!isOpen) {
                item.classList.add('open');
            }
        });
    });
});
