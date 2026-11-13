# nodewipe-action

Official GitHub Action for [nodewipe](https://github.com/KADHIRAVANEG/nodewipe) — fast CI/CD runner disk space recovery and artifact cache pruning across 13+ ecosystems.

GitHub Actions runners (especially standard 14GB Ubuntu runners) frequently exhaust available storage during heavy workflows (Docker builds, monorepos, Rust targets, Android SDK caches). `nodewipe-action` reclaims storage before, between, or after pipeline steps.

## Usage

```yaml
- name: Reclaim CI Runner Disk Space
  uses: KADHIRAVANEG/nodewipe-action@v1
  with:
    path: '.'
    mode: 'delete'
    older-than: '24h'
```

### Free Up Space Before Docker Builds

```yaml
steps:
  - uses: actions/checkout@v4

  # Wipe old host caches before Docker builds
  - name: Free Disk Space
    uses: KADHIRAVANEG/nodewipe-action@v1
    with:
      path: '~'
      mode: 'delete'

  - name: Build Docker Image
    run: docker build -t my-app .
```

### Dry-Run Analysis in CI

```yaml
- name: Analyze Artifact Disk Waste
  uses: KADHIRAVANEG/nodewipe-action@v1
  with:
    path: '.'
    mode: 'dry-run'
```

## Inputs

| Input | Description | Default |
| :--- | :--- | :--- |
| `version` | Version of nodewipe to install (`v0.9.0` or `latest`) | `v0.9.0` |
| `path` | Path to directory to scan and clean | `.` |
| `mode` | Operation mode: `delete`, `dry-run`, or `dedupe` | `delete` |
| `older-than` | Filter artifacts older than duration (e.g. `24h`, `7d`) | `""` |
| `exclude-types`| Comma-separated artifact types to skip | `""` |
| `summary` | Output Markdown summary to GitHub Actions job summary | `true` |
