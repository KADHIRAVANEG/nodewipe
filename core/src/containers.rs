//! Container and virtualization build cache excision engine (Docker, Podman, Colima).
//!
//! Communicates over local Unix domain sockets (or fallback CLI/Windows pipe checks)
//! to safely enumerate and prune dangling BuildKit caches, untagged intermediate image
//! layers, and unattached anonymous volumes without affecting running containers,
//! tagged images, or named persistent volumes.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

/// Supported local container and virtualization runtimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerProviderKind {
    Docker,
    Podman,
    Colima,
}

impl ContainerProviderKind {
    pub fn label(&self) -> &'static str {
        match self {
            ContainerProviderKind::Docker => "Docker",
            ContainerProviderKind::Podman => "Podman",
            ContainerProviderKind::Colima => "Colima",
        }
    }

    pub fn from_slug(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "docker" => Some(Self::Docker),
            "podman" => Some(Self::Podman),
            "colima" => Some(Self::Colima),
            _ => None,
        }
    }
}

/// Classification of reclaimable container runtime storage objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerCacheKind {
    BuildKitCache,
    DanglingImageLayer,
    AnonymousVolume,
    StoppedContainer,
}

impl ContainerCacheKind {
    pub fn label(&self) -> &'static str {
        match self {
            ContainerCacheKind::BuildKitCache => "BuildKit Cache",
            ContainerCacheKind::DanglingImageLayer => "Image Layer",
            ContainerCacheKind::AnonymousVolume => "Volume",
            ContainerCacheKind::StoppedContainer => "Stopped Container",
        }
    }
}

/// A discovered container build cache, dangling layer, or volume entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerArtifact {
    pub id: String,
    pub short_id: String,
    pub provider: ContainerProviderKind,
    pub kind: ContainerCacheKind,
    pub description: String,
    pub size_bytes: u64,
    pub formatted_size: String,
    pub created_unix: Option<u64>,
    pub last_used_unix: Option<u64>,
    pub in_use: bool,
    pub named: bool,
    pub reclaimable: bool,
}

/// Scan report produced for an individual container runtime adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerScanReport {
    pub provider: ContainerProviderKind,
    pub endpoint: String,
    pub available: bool,
    pub status_message: String,
    pub total_reclaimable_bytes: u64,
    pub formatted_reclaimable: String,
    pub running_containers_count: usize,
    pub artifacts: Vec<ContainerArtifact>,
}

/// Prune outcome produced for a container runtime adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerPruneReport {
    pub provider: ContainerProviderKind,
    pub endpoint: String,
    pub dry_run: bool,
    pub available: bool,
    pub message: String,
    pub pruned_items: usize,
    pub skipped_protected_items: usize,
    pub reclaimed_bytes: u64,
    pub formatted_reclaimed: String,
    pub targets: Vec<ContainerArtifact>,
}

/// Options controlling container cache discovery and safety boundaries.
#[derive(Debug, Clone, Default)]
pub struct ContainerScanOptions {
    /// Include unused tagged images & stopped containers in reclaimable targets (`--all`).
    pub include_all: bool,
    /// Allow pruning named persistent volumes when unattached (`--force-all`).
    pub force_all: bool,
    /// Restrict inspection to a specific provider runtime.
    pub provider_filter: Option<ContainerProviderKind>,
    /// Explicit socket endpoints (primarily for custom daemon paths and automated tests).
    /// When empty, standard platform endpoints are auto-discovered.
    pub custom_sockets: Vec<(ContainerProviderKind, String)>,
}

/// Pluggable container runtime inspection & pruning abstraction.
pub trait ContainerCleaner: Send + Sync {
    fn provider(&self) -> ContainerProviderKind;
    fn endpoint(&self) -> &str;
    fn ping(&self) -> Result<()>;
    fn scan(&self, opts: &ContainerScanOptions) -> ContainerScanReport;
    fn prune(&self, opts: &ContainerScanOptions, dry_run: bool) -> ContainerPruneReport;
}

/// Adapter for Docker Engine, Docker Desktop, and Colima VM Unix sockets.
#[derive(Debug, Clone)]
pub struct DockerEngineAdapter {
    provider: ContainerProviderKind,
    endpoint: String,
}

impl DockerEngineAdapter {
    pub fn new(provider: ContainerProviderKind, endpoint: impl Into<String>) -> Self {
        Self {
            provider,
            endpoint: endpoint.into(),
        }
    }
}

/// Adapter for rootless and system Podman API Unix sockets.
#[derive(Debug, Clone)]
pub struct PodmanAdapter {
    endpoint: String,
}

impl PodmanAdapter {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }
}

impl ContainerCleaner for DockerEngineAdapter {
    fn provider(&self) -> ContainerProviderKind {
        self.provider
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn ping(&self) -> Result<()> {
        let (_, body) = unix_http_request(&self.endpoint, "GET", "/_ping")?;
        if body.trim().eq_ignore_ascii_case("OK") || body.is_empty() {
            Ok(())
        } else {
            bail!("unexpected ping response: {body}")
        }
    }

    fn scan(&self, opts: &ContainerScanOptions) -> ContainerScanReport {
        scan_docker_compatible_socket(self.provider, &self.endpoint, opts)
    }

    fn prune(&self, opts: &ContainerScanOptions, dry_run: bool) -> ContainerPruneReport {
        prune_docker_compatible_socket(self.provider, &self.endpoint, opts, dry_run)
    }
}

impl ContainerCleaner for PodmanAdapter {
    fn provider(&self) -> ContainerProviderKind {
        ContainerProviderKind::Podman
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn ping(&self) -> Result<()> {
        let (_, body) = unix_http_request(&self.endpoint, "GET", "/_ping")?;
        if body.trim().eq_ignore_ascii_case("OK") || body.is_empty() {
            Ok(())
        } else {
            bail!("unexpected ping response: {body}")
        }
    }

    fn scan(&self, opts: &ContainerScanOptions) -> ContainerScanReport {
        scan_docker_compatible_socket(ContainerProviderKind::Podman, &self.endpoint, opts)
    }

    fn prune(&self, opts: &ContainerScanOptions, dry_run: bool) -> ContainerPruneReport {
        prune_docker_compatible_socket(ContainerProviderKind::Podman, &self.endpoint, opts, dry_run)
    }
}

/// Discovers candidate container runtime adapters present on the host system.
pub fn discover_container_adapters(opts: &ContainerScanOptions) -> Vec<Box<dyn ContainerCleaner>> {
    if !opts.custom_sockets.is_empty() {
        return opts
            .custom_sockets
            .iter()
            .filter(|(p, _)| opts.provider_filter.is_none_or(|f| f == *p))
            .map(|(p, ep)| -> Box<dyn ContainerCleaner> {
                match p {
                    ContainerProviderKind::Podman => Box::new(PodmanAdapter::new(ep.clone())),
                    other => Box::new(DockerEngineAdapter::new(*other, ep.clone())),
                }
            })
            .collect();
    }

    let mut adapters: Vec<Box<dyn ContainerCleaner>> = Vec::new();
    let mut seen_endpoints: HashSet<String> = HashSet::new();

    fn push_adapter(
        adapters: &mut Vec<Box<dyn ContainerCleaner>>,
        seen: &mut HashSet<String>,
        filter: Option<ContainerProviderKind>,
        provider: ContainerProviderKind,
        ep: String,
    ) {
        if filter.is_some_and(|f| f != provider) {
            return;
        }
        if seen.insert(ep.clone()) {
            match provider {
                ContainerProviderKind::Podman => adapters.push(Box::new(PodmanAdapter::new(ep))),
                other => adapters.push(Box::new(DockerEngineAdapter::new(other, ep))),
            }
        }
    }

    if let Ok(dh) = std::env::var("DOCKER_HOST")
        && let Some(sock) = dh.strip_prefix("unix://")
    {
        let prov = if sock.contains("colima") {
            ContainerProviderKind::Colima
        } else if sock.contains("podman") {
            ContainerProviderKind::Podman
        } else {
            ContainerProviderKind::Docker
        };
        push_adapter(
            &mut adapters,
            &mut seen_endpoints,
            opts.provider_filter,
            prov,
            sock.to_string(),
        );
    }

    #[cfg(unix)]
    {
        push_adapter(
            &mut adapters,
            &mut seen_endpoints,
            opts.provider_filter,
            ContainerProviderKind::Docker,
            "/var/run/docker.sock".to_string(),
        );

        if let Some(home) = dirs::home_dir() {
            let desktop_sock = home.join(".docker/desktop/docker.sock");
            if desktop_sock.exists() {
                push_adapter(
                    &mut adapters,
                    &mut seen_endpoints,
                    opts.provider_filter,
                    ContainerProviderKind::Docker,
                    desktop_sock.to_string_lossy().to_string(),
                );
            }
            let colima_sock = home.join(".colima/default/docker.sock");
            if colima_sock.exists() {
                push_adapter(
                    &mut adapters,
                    &mut seen_endpoints,
                    opts.provider_filter,
                    ContainerProviderKind::Colima,
                    colima_sock.to_string_lossy().to_string(),
                );
            }
        }

        let uid = std::env::var("UID").ok().or_else(|| {
            std::env::var("XDG_RUNTIME_DIR")
                .ok()
                .and_then(|d| d.rsplit('/').next().map(ToString::to_string))
        });

        if let Some(u) = uid {
            let rootless_docker = format!("/run/user/{u}/docker.sock");
            if PathBuf::from(&rootless_docker).exists() {
                push_adapter(
                    &mut adapters,
                    &mut seen_endpoints,
                    opts.provider_filter,
                    ContainerProviderKind::Docker,
                    rootless_docker,
                );
            }

            let podman_user = format!("/run/user/{u}/podman/podman.sock");
            push_adapter(
                &mut adapters,
                &mut seen_endpoints,
                opts.provider_filter,
                ContainerProviderKind::Podman,
                podman_user,
            );
        }

        let podman_sys = "/run/podman/podman.sock".to_string();
        if PathBuf::from(&podman_sys).exists() {
            push_adapter(
                &mut adapters,
                &mut seen_endpoints,
                opts.provider_filter,
                ContainerProviderKind::Podman,
                podman_sys,
            );
        }
    }

    #[cfg(windows)]
    {
        push_adapter(
            &mut adapters,
            &mut seen_endpoints,
            opts.provider_filter,
            ContainerProviderKind::Docker,
            r"\\.\pipe\docker_engine".to_string(),
        );
    }

    adapters
}

/// Scans all discovered container runtimes cleanly, falling back gracefully if sockets are offline.
pub fn scan_container_caches(opts: &ContainerScanOptions) -> Vec<ContainerScanReport> {
    let adapters = discover_container_adapters(opts);
    adapters.iter().map(|a| a.scan(opts)).collect()
}

/// Prunes container build caches across discovered runtimes while preserving safety invariants.
pub fn prune_container_caches(
    opts: &ContainerScanOptions,
    dry_run: bool,
) -> Vec<ContainerPruneReport> {
    let adapters = discover_container_adapters(opts);
    adapters
        .iter()
        .filter_map(|a| {
            let scan_rep = a.scan(opts);
            if !scan_rep.available {
                return None;
            }
            Some(a.prune(opts, dry_run))
        })
        .collect()
}

// ── Minimal HTTP/1.1 over Unix Domain Socket Client ───────────────────────────────

const MAX_HTTP_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;

fn unix_http_request(endpoint: &str, method: &str, path: &str) -> Result<(u16, String)> {
    if method.chars().any(|c| c.is_control() || c.is_whitespace()) {
        bail!("invalid HTTP method: contains control or whitespace characters");
    }
    if path.chars().any(|c| c.is_control() || c.is_whitespace()) {
        bail!("invalid HTTP request path: contains control or whitespace characters");
    }

    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;

        let sock_path = endpoint.strip_prefix("unix://").unwrap_or(endpoint);
        let mut stream = UnixStream::connect(sock_path)
            .with_context(|| format!("failed to connect to container socket {sock_path}"))?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));

        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
        );
        stream.write_all(req.as_bytes())?;

        let mut raw = Vec::new();
        stream
            .take(MAX_HTTP_RESPONSE_BYTES + 1)
            .read_to_end(&mut raw)?;
        if raw.len() as u64 > MAX_HTTP_RESPONSE_BYTES {
            bail!("HTTP response from container socket exceeded 32 MiB limit");
        }

        parse_http_response(&raw)
    }

    #[cfg(not(unix))]
    {
        let _ = (endpoint, method, path);
        bail!("Unix domain socket HTTP client unavailable on non-Unix host")
    }
}

fn parse_http_response(raw: &[u8]) -> Result<(u16, String)> {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("malformed HTTP response from container socket")?;
    let headers_str = String::from_utf8_lossy(&raw[..sep]);
    let status_line = headers_str.lines().next().unwrap_or("");
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .unwrap_or(500);

    let body_raw = &raw[sep + 4..];
    let is_chunked = headers_str.lines().any(|l| {
        l.to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
    });

    let body_bytes = if is_chunked {
        decode_chunked_body(body_raw)?
    } else {
        body_raw.to_vec()
    };

    Ok((
        status_code,
        String::from_utf8_lossy(&body_bytes).to_string(),
    ))
}

fn decode_chunked_body(mut data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let Some(crlf) = data.windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let size_hex = String::from_utf8_lossy(&data[..crlf]);
        let clean_hex = size_hex.split(';').next().unwrap_or("").trim();
        let chunk_len = usize::from_str_radix(clean_hex, 16).unwrap_or(0);
        data = &data[crlf + 2..];
        if chunk_len == 0 || data.len() < chunk_len {
            break;
        }
        out.extend_from_slice(&data[..chunk_len]);
        data = &data[chunk_len..];
        if data.starts_with(b"\r\n") {
            data = &data[2..];
        }
    }
    Ok(out)
}

fn format_bytes(bytes: u64) -> String {
    crate::explain::format_size(bytes)
}

fn short_id(id: &str) -> String {
    let stripped = id.strip_prefix("sha256:").unwrap_or(id);
    stripped.chars().take(12).collect()
}

fn is_anonymous_volume_name(name: &str) -> bool {
    name.len() == 64 && name.chars().all(|c| c.is_ascii_hexdigit())
}

fn parse_rfc3339_or_unix_opt(v: Option<&Value>) -> Option<u64> {
    let val = v?;
    if let Some(n) = val.as_u64() {
        return Some(n);
    }
    if let Some(n) = val.as_i64()
        && n > 0
    {
        return Some(n as u64);
    }
    None
}

fn scan_docker_compatible_socket(
    provider: ContainerProviderKind,
    endpoint: &str,
    opts: &ContainerScanOptions,
) -> ContainerScanReport {
    let offline = |msg: String| ContainerScanReport {
        provider,
        endpoint: endpoint.to_string(),
        available: false,
        status_message: msg,
        total_reclaimable_bytes: 0,
        formatted_reclaimable: "0 B".to_string(),
        running_containers_count: 0,
        artifacts: Vec::new(),
    };

    let sock_path = endpoint.strip_prefix("unix://").unwrap_or(endpoint);
    if !sock_path.starts_with(r"\\.\pipe\") && !PathBuf::from(sock_path).exists() {
        return offline(format!("Socket not present at {sock_path}"));
    }

    match unix_http_request(endpoint, "GET", "/_ping") {
        Ok((code, _)) if code < 400 => {}
        Ok((code, body)) => {
            return offline(format!("Daemon ping returned HTTP {code}: {body}"));
        }
        Err(e) => {
            return offline(format!("Daemon unreachable ({e})"));
        }
    }

    // Check running containers & mounted images/volumes so running state is never touched
    let mut running_count = 0usize;
    let mut active_images: HashSet<String> = HashSet::new();
    let mut active_volumes: HashSet<String> = HashSet::new();

    if let Ok((200, containers_json)) =
        unix_http_request(endpoint, "GET", "/containers/json?all=true")
        && let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(&containers_json)
    {
        for c in arr {
            let state = c.get("State").and_then(|v| v.as_str()).unwrap_or("");
            if state.eq_ignore_ascii_case("running") {
                running_count += 1;
                if let Some(img_id) = c.get("ImageID").and_then(|v| v.as_str()) {
                    active_images.insert(img_id.to_string());
                }
            }
            if let Some(Value::Array(mounts)) = c.get("Mounts") {
                for m in mounts {
                    if let Some(name) = m.get("Name").and_then(|v| v.as_str()) {
                        active_volumes.insert(name.to_string());
                    }
                }
            }
        }
    }

    let (df_code, df_body) = match unix_http_request(endpoint, "GET", "/system/df") {
        Ok(res) => res,
        Err(e) => return offline(format!("Failed querying /system/df: {e}")),
    };
    if df_code >= 400 {
        return offline(format!("/system/df failed with HTTP {df_code}: {df_body}"));
    }

    let df_val: Value = match serde_json::from_str(&df_body) {
        Ok(v) => v,
        Err(e) => return offline(format!("Invalid /system/df JSON payload: {e}")),
    };

    let mut artifacts = Vec::new();

    // 1. BuildKit cache records
    if let Some(Value::Array(build_caches)) = df_val.get("BuildCache") {
        for bc in build_caches {
            let id = bc
                .get("ID")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let in_use = bc.get("InUse").and_then(|v| v.as_bool()).unwrap_or(false);
            let shared = bc.get("Shared").and_then(|v| v.as_bool()).unwrap_or(false);
            let size = bc.get("Size").and_then(|v| v.as_u64()).unwrap_or(0);
            let desc = bc
                .get("Description")
                .and_then(|v| v.as_str())
                .unwrap_or("BuildKit cache layer")
                .to_string();
            let created_unix = parse_rfc3339_or_unix_opt(bc.get("CreatedAt"));
            let last_used_unix = parse_rfc3339_or_unix_opt(bc.get("LastUsedAt"));
            let reclaimable = !in_use && (!shared || opts.include_all);

            artifacts.push(ContainerArtifact {
                short_id: short_id(&id),
                id,
                provider,
                kind: ContainerCacheKind::BuildKitCache,
                description: desc,
                size_bytes: size,
                formatted_size: format_bytes(size),
                created_unix,
                last_used_unix,
                in_use,
                named: false,
                reclaimable,
            });
        }
    }

    // 2. Dangling / untagged intermediate images
    if let Some(Value::Array(images)) = df_val.get("Images") {
        for img in images {
            let id = img
                .get("Id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let containers = img.get("Containers").and_then(|v| v.as_i64()).unwrap_or(0);
            let tags: Vec<&str> = img
                .get("RepoTags")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|t| t.as_str()).collect())
                .unwrap_or_default();

            let is_dangling = tags.is_empty() || tags.iter().all(|t| *t == "<none>:<none>");
            let in_use = containers > 0 || active_images.contains(&id);
            let size = img.get("Size").and_then(|v| v.as_u64()).unwrap_or(0);
            let created_unix = parse_rfc3339_or_unix_opt(img.get("Created"));

            let desc = if is_dangling {
                "Dangling untagged image layer (<none>:<none>)".to_string()
            } else {
                format!("Image {}", tags.join(", "))
            };
            let reclaimable = !in_use && (is_dangling || opts.include_all);

            artifacts.push(ContainerArtifact {
                short_id: short_id(&id),
                id,
                provider,
                kind: ContainerCacheKind::DanglingImageLayer,
                description: desc,
                size_bytes: size,
                formatted_size: format_bytes(size),
                created_unix,
                last_used_unix: None,
                in_use,
                named: !is_dangling,
                reclaimable,
            });
        }
    }

    // 3. Unattached anonymous volumes (and named volumes only if force_all)
    if let Some(Value::Array(volumes)) = df_val.get("Volumes") {
        for vol in volumes {
            let name = vol
                .get("Name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let usage = vol.get("UsageData");
            let ref_count = usage
                .and_then(|u| u.get("RefCount"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let size = usage
                .and_then(|u| u.get("Size"))
                .and_then(|v| v.as_i64())
                .map(|s| s.max(0) as u64)
                .unwrap_or(0);

            let in_use = ref_count > 0 || active_volumes.contains(&name);
            let is_anon = is_anonymous_volume_name(&name);
            let named = !is_anon;
            let reclaimable = !in_use && (is_anon || opts.force_all);

            artifacts.push(ContainerArtifact {
                short_id: short_id(&name),
                id: name.clone(),
                provider,
                kind: ContainerCacheKind::AnonymousVolume,
                description: if named {
                    format!("Named volume: {name}")
                } else {
                    "Unattached anonymous container volume".to_string()
                },
                size_bytes: size,
                formatted_size: format_bytes(size),
                created_unix: None,
                last_used_unix: None,
                in_use,
                named,
                reclaimable,
            });
        }
    }

    // 4. Stopped containers (reclaimable only when include_all is enabled)
    if opts.include_all
        && let Some(Value::Array(containers)) = df_val.get("Containers")
    {
        for c in containers {
            let id = c
                .get("Id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let state = c.get("State").and_then(|v| v.as_str()).unwrap_or("");
            let is_running = state.eq_ignore_ascii_case("running");
            if is_running {
                continue;
            }
            let size = c.get("SizeRw").and_then(|v| v.as_u64()).unwrap_or(0);
            let img = c.get("Image").and_then(|v| v.as_str()).unwrap_or("unknown");

            artifacts.push(ContainerArtifact {
                short_id: short_id(&id),
                id,
                provider,
                kind: ContainerCacheKind::StoppedContainer,
                description: format!("Stopped container ({img})"),
                size_bytes: size,
                formatted_size: format_bytes(size),
                created_unix: parse_rfc3339_or_unix_opt(c.get("Created")),
                last_used_unix: None,
                in_use: false,
                named: false,
                reclaimable: true,
            });
        }
    }

    artifacts.sort_by_key(|a| std::cmp::Reverse(a.size_bytes));
    let total_reclaimable_bytes: u64 = artifacts
        .iter()
        .filter(|a| a.reclaimable)
        .map(|a| a.size_bytes)
        .sum();

    ContainerScanReport {
        provider,
        endpoint: endpoint.to_string(),
        available: true,
        status_message: "Connected".to_string(),
        total_reclaimable_bytes,
        formatted_reclaimable: format_bytes(total_reclaimable_bytes),
        running_containers_count: running_count,
        artifacts,
    }
}

fn prune_docker_compatible_socket(
    provider: ContainerProviderKind,
    endpoint: &str,
    opts: &ContainerScanOptions,
    dry_run: bool,
) -> ContainerPruneReport {
    let scan_rep = scan_docker_compatible_socket(provider, endpoint, opts);
    if !scan_rep.available {
        return ContainerPruneReport {
            provider,
            endpoint: endpoint.to_string(),
            dry_run,
            available: false,
            message: scan_rep.status_message,
            pruned_items: 0,
            skipped_protected_items: 0,
            reclaimed_bytes: 0,
            formatted_reclaimed: "0 B".to_string(),
            targets: Vec::new(),
        };
    }

    let targets: Vec<ContainerArtifact> = scan_rep
        .artifacts
        .iter()
        .filter(|a| a.reclaimable)
        .cloned()
        .collect();
    let skipped_protected_items = scan_rep.artifacts.len().saturating_sub(targets.len());

    if dry_run {
        let total: u64 = targets.iter().map(|a| a.size_bytes).sum();
        return ContainerPruneReport {
            provider,
            endpoint: endpoint.to_string(),
            dry_run: true,
            available: true,
            message: format!(
                "Dry-run preview: {} item(s) eligible ({} protected)",
                targets.len(),
                skipped_protected_items
            ),
            pruned_items: targets.len(),
            skipped_protected_items,
            reclaimed_bytes: total,
            formatted_reclaimed: format_bytes(total),
            targets,
        };
    }

    let mut total_reclaimed = 0u64;
    let mut messages = Vec::new();

    if targets
        .iter()
        .any(|t| t.kind == ContainerCacheKind::BuildKitCache)
    {
        let build_path = if opts.include_all {
            "/build/prune?all=true"
        } else {
            "/build/prune"
        };
        if let Ok((code, body)) = unix_http_request(endpoint, "POST", build_path)
            && code < 400
        {
            if let Ok(val) = serde_json::from_str::<Value>(&body)
                && let Some(reclaimed) = val.get("SpaceReclaimed").and_then(|v| v.as_u64())
            {
                total_reclaimed = total_reclaimed.saturating_add(reclaimed);
            }
            messages.push("BuildKit cache pruned");
        }
    }

    if targets
        .iter()
        .any(|t| t.kind == ContainerCacheKind::DanglingImageLayer)
    {
        let img_path = if opts.include_all {
            "/images/prune?filters=%7B%22dangling%22%3A%5B%22false%22%5D%7D"
        } else {
            "/images/prune?filters=%7B%22dangling%22%3A%5B%22true%22%5D%7D"
        };
        if let Ok((code, body)) = unix_http_request(endpoint, "POST", img_path)
            && code < 400
        {
            if let Ok(val) = serde_json::from_str::<Value>(&body)
                && let Some(reclaimed) = val.get("SpaceReclaimed").and_then(|v| v.as_u64())
            {
                total_reclaimed = total_reclaimed.saturating_add(reclaimed);
            }
            messages.push("Dangling images pruned");
        }
    }

    if targets
        .iter()
        .any(|t| t.kind == ContainerCacheKind::AnonymousVolume)
    {
        let vol_path = if opts.force_all {
            "/volumes/prune?filters=%7B%22all%22%3A%5B%22true%22%5D%7D"
        } else {
            "/volumes/prune"
        };
        if let Ok((code, body)) = unix_http_request(endpoint, "POST", vol_path)
            && code < 400
        {
            if let Ok(val) = serde_json::from_str::<Value>(&body)
                && let Some(reclaimed) = val.get("SpaceReclaimed").and_then(|v| v.as_u64())
            {
                total_reclaimed = total_reclaimed.saturating_add(reclaimed);
            }
            messages.push("Unattached volumes pruned");
        }
    }

    if opts.include_all
        && targets
            .iter()
            .any(|t| t.kind == ContainerCacheKind::StoppedContainer)
        && let Ok((code, body)) = unix_http_request(endpoint, "POST", "/containers/prune")
        && code < 400
    {
        if let Ok(val) = serde_json::from_str::<Value>(&body)
            && let Some(reclaimed) = val.get("SpaceReclaimed").and_then(|v| v.as_u64())
        {
            total_reclaimed = total_reclaimed.saturating_add(reclaimed);
        }
        messages.push("Stopped containers pruned");
    }

    // Fallback to scanned estimate if mock/daemon returned 0 SpaceReclaimed
    if total_reclaimed == 0 && !targets.is_empty() {
        total_reclaimed = targets.iter().map(|t| t.size_bytes).sum();
    }

    ContainerPruneReport {
        provider,
        endpoint: endpoint.to_string(),
        dry_run: false,
        available: true,
        message: if messages.is_empty() {
            "No reclaimable container artifacts".to_string()
        } else {
            messages.join(", ")
        },
        pruned_items: targets.len(),
        skipped_protected_items,
        reclaimed_bytes: total_reclaimed,
        formatted_reclaimed: format_bytes(total_reclaimed),
        targets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offline_socket_graceful_fallback() {
        let opts = ContainerScanOptions {
            custom_sockets: vec![(
                ContainerProviderKind::Docker,
                "/tmp/nodewipe-nonexistent-docker.sock".to_string(),
            )],
            ..Default::default()
        };

        let reports = scan_container_caches(&opts);
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].available);
        assert_eq!(reports[0].total_reclaimable_bytes, 0);
        assert!(reports[0].status_message.contains("Socket not present"));
    }

    #[test]
    #[cfg(unix)]
    fn test_mock_docker_unix_socket_scan_safety_and_prune() {
        use std::os::unix::net::UnixListener;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use tempfile::tempdir;

        let tmp = tempdir().unwrap();
        let sock_path = tmp.path().join("mock-docker.sock");
        let listener = UnixListener::bind(&sock_path).unwrap();

        let build_pruned = Arc::new(AtomicBool::new(false));
        let images_pruned = Arc::new(AtomicBool::new(false));
        let volumes_pruned = Arc::new(AtomicBool::new(false));

        let bp = build_pruned.clone();
        let ip = images_pruned.clone();
        let vp = volumes_pruned.clone();

        let server = std::thread::spawn(move || {
            for stream_res in listener.incoming() {
                let mut stream = match stream_res {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    continue;
                }
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("");
                let path = parts.next().unwrap_or("");

                if path == "/_ping" {
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK");
                } else if path.starts_with("/containers/json") {
                    let body = r#"[{"Id":"run1","State":"running","ImageID":"img_active","Mounts":[{"Name":"pg_data"}]}]"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else if path == "/system/df" {
                    let anon_vol = "a".repeat(64);
                    let body = serde_json::json!({
                        "BuildCache": [
                            {"ID": "bc_stale", "InUse": false, "Shared": false, "Size": 104857600, "Description": "RUN npm install"},
                            {"ID": "bc_live", "InUse": true, "Shared": false, "Size": 52428800, "Description": "Active stage"}
                        ],
                        "Images": [
                            {"Id": "img_dangling", "RepoTags": ["<none>:<none>"], "Containers": 0, "Size": 20971520},
                            {"Id": "img_tagged", "RepoTags": ["node:22-alpine"], "Containers": 0, "Size": 80000000}
                        ],
                        "Volumes": [
                            {"Name": anon_vol, "UsageData": {"RefCount": 0, "Size": 10485760}},
                            {"Name": "named_db_vol", "UsageData": {"RefCount": 0, "Size": 50000000}}
                        ]
                    })
                    .to_string();
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else if method == "POST" && path.starts_with("/build/prune") {
                    bp.store(true, Ordering::SeqCst);
                    let body = r#"{"SpaceReclaimed":104857600}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else if method == "POST" && path.starts_with("/images/prune") {
                    ip.store(true, Ordering::SeqCst);
                    let body = r#"{"SpaceReclaimed":20971520}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else if method == "POST" && path.starts_with("/volumes/prune") {
                    vp.store(true, Ordering::SeqCst);
                    let body = r#"{"SpaceReclaimed":10485760}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else if path == "/stop" {
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK");
                    break;
                }
            }
        });

        let ep = sock_path.to_string_lossy().to_string();
        let opts = ContainerScanOptions {
            include_all: false,
            force_all: false,
            provider_filter: Some(ContainerProviderKind::Docker),
            custom_sockets: vec![(ContainerProviderKind::Docker, ep.clone())],
        };

        // 1. Scan verifies safety classification
        let reports = scan_container_caches(&opts);
        assert_eq!(reports.len(), 1);
        assert!(reports[0].available);
        assert_eq!(reports[0].running_containers_count, 1);
        // Only bc_stale (100 MiB) + img_dangling (20 MiB) + anon_vol (10 MiB) = 136_314_880 B
        assert_eq!(reports[0].total_reclaimable_bytes, 136_314_880);

        let named_vol = reports[0]
            .artifacts
            .iter()
            .find(|a| a.id == "named_db_vol")
            .unwrap();
        assert!(named_vol.named);
        assert!(
            !named_vol.reclaimable,
            "named volumes must be protected by default"
        );

        let tagged_img = reports[0]
            .artifacts
            .iter()
            .find(|a| a.id == "img_tagged")
            .unwrap();
        assert!(
            !tagged_img.reclaimable,
            "tagged images must be protected without --all"
        );

        // 2. Dry-run prune must NOT issue mutating POST calls
        let dry = prune_container_caches(&opts, true);
        assert_eq!(dry.len(), 1);
        assert!(dry[0].dry_run);
        assert_eq!(dry[0].pruned_items, 3);
        assert!(!build_pruned.load(Ordering::SeqCst));
        assert!(!images_pruned.load(Ordering::SeqCst));
        assert!(!volumes_pruned.load(Ordering::SeqCst));

        // 3. Live prune issues POST prune endpoints
        let live = prune_container_caches(&opts, false);
        assert_eq!(live.len(), 1);
        assert!(!live[0].dry_run);
        assert_eq!(live[0].reclaimed_bytes, 136_314_880);
        assert!(build_pruned.load(Ordering::SeqCst));
        assert!(images_pruned.load(Ordering::SeqCst));
        assert!(volumes_pruned.load(Ordering::SeqCst));

        let _ = unix_http_request(&ep, "GET", "/stop");
        let _ = server.join();
    }

    #[test]
    fn test_unix_http_request_rejects_crlf_smuggling() {
        assert!(unix_http_request("/tmp/bogus.sock", "GET\r\nEvil: 1", "/info").is_err());
        assert!(
            unix_http_request("/tmp/bogus.sock", "GET", "/info HTTP/1.1\r\nHost: evil").is_err()
        );
        assert!(unix_http_request("/tmp/bogus.sock", "GET", "/info\0").is_err());
    }

    #[test]
    fn test_decode_chunked_body_resists_malformed_lengths() {
        assert!(decode_chunked_body(b"ZZZZ\r\nabcd\r\n0\r\n\r\n").is_ok());
        assert!(decode_chunked_body(b"ffff\r\nshort").is_ok());
    }
}
