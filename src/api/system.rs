use rocket::{State, get, serde::json::Json};
use serde::Serialize;
use sqlx::SqlitePool;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use super::errors::{ApiResult, internal};
use crate::db::settings as db_settings;

#[derive(Serialize)]
pub struct VersionInfo {
    pub version: String,
    pub build_type: String,
    pub git_commit: Option<String>,
}

#[derive(Serialize)]
pub struct SystemInfo {
    /// RSS memory used by this process and all its descendants (MB).
    pub process_mem_mb: u64,
    /// Number of manga in the library.
    pub db_manga_count: i64,
    /// Total chapters scraped.
    pub db_chapter_count: i64,
    /// Chapters with status = Downloaded.
    pub db_downloaded_count: i64,
    /// Tasks with status = Pending.
    pub tasks_pending: i64,
    /// Tasks with status = Running.
    pub tasks_running: i64,
    /// Whether the background task queue is paused.
    pub queue_paused: bool,
}

#[derive(Serialize)]
pub struct DesktopHealth {
    /// True when Xvfb display socket exists (`/tmp/.X11-unix/X99`).
    pub xvfb: bool,
    /// True when local x11vnc TCP listener is reachable.
    pub vnc: bool,
    /// True when local noVNC/websockify TCP listener is reachable.
    pub novnc: bool,
}

/// Build a map of ppid→[child_pids] by scanning /proc/*/stat.
fn build_children_map() -> std::collections::HashMap<u32, Vec<u32>> {
    let mut children: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    let Ok(proc_dir) = std::fs::read_dir("/proc") else {
        return children;
    };
    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let Ok(pid) = name.to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        // Format: pid (comm) state ppid ... — comm may contain spaces/parens, find last ')'
        let Some(after_comm) = stat.rfind(')') else {
            continue;
        };
        let fields: Vec<&str> = stat[after_comm + 1..].split_whitespace().collect();
        if fields.len() >= 2 {
            if let Ok(ppid) = fields[1].parse::<u32>() {
                children.entry(ppid).or_default().push(pid);
            }
        }
    }
    children
}

/// Collect all PIDs in the subtree rooted at `root_pid` (inclusive).
fn collect_process_tree(root_pid: u32) -> Vec<u32> {
    let children = build_children_map();
    let mut result = Vec::new();
    let mut stack = vec![root_pid];
    while let Some(pid) = stack.pop() {
        result.push(pid);
        if let Some(kids) = children.get(&pid) {
            stack.extend_from_slice(kids);
        }
    }
    result
}

/// Read VmRSS (kB) for a single PID from /proc/<pid>/status.
fn read_vmrss_kb(pid: u32) -> u64 {
    let Ok(content) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return 0;
    };
    for line in content.lines() {
        if line.starts_with("VmRSS:") {
            return line
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        }
    }
    0
}

/// Total RSS (MB) for this process and all its descendants (including Chromium children).
fn process_tree_rss_mb() -> u64 {
    let pids = collect_process_tree(std::process::id());
    pids.iter().map(|&pid| read_vmrss_kb(pid)).sum::<u64>() / 1024
}

fn tcp_listening_on_local(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

// ---------------------------------------------------------------------------
// GET /api/system
// ---------------------------------------------------------------------------

#[get("/api/system")]
pub async fn system_info(pool: &State<SqlitePool>) -> ApiResult<SystemInfo> {
    let process_mem_mb = process_tree_rss_mb();

    let manga_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM Manga")
        .fetch_one(pool.inner())
        .await
        .map_err(internal)?;

    let chapter_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM Chapters")
        .fetch_one(pool.inner())
        .await
        .map_err(internal)?;

    let downloaded_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM Chapters WHERE download_status = 'Downloaded'")
            .fetch_one(pool.inner())
            .await
            .map_err(internal)?;

    let pending_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM Task WHERE status = 'Pending'")
            .fetch_one(pool.inner())
            .await
            .map_err(internal)?;

    let running_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM Task WHERE status = 'Running'")
            .fetch_one(pool.inner())
            .await
            .map_err(internal)?;

    let queue_paused = db_settings::get(pool.inner(), "queue_paused", "false")
        .await
        .unwrap_or_else(|_| "false".to_string())
        == "true";

    Ok(Json(SystemInfo {
        process_mem_mb,
        db_manga_count: manga_count.0,
        db_chapter_count: chapter_count.0,
        db_downloaded_count: downloaded_count.0,
        tasks_pending: pending_count.0,
        tasks_running: running_count.0,
        queue_paused,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/system/desktop
// ---------------------------------------------------------------------------

#[get("/api/system/desktop")]
pub async fn desktop_health() -> Json<DesktopHealth> {
    Json(DesktopHealth {
        xvfb: std::path::Path::new("/tmp/.X11-unix/X99").exists(),
        vnc: tcp_listening_on_local(5900),
        novnc: tcp_listening_on_local(16080),
    })
}

// ---------------------------------------------------------------------------
// GET /api/version
// ---------------------------------------------------------------------------

#[get("/api/version")]
pub fn version_info() -> Json<VersionInfo> {
    // Determine build type based on environment variables
    let build_type = if cfg!(debug_assertions) {
        "dev".to_string()
    } else if std::env::var("GITLAB_CI").is_ok() {
        "gitlab-ci".to_string()
    } else if std::env::var("CI").is_ok() {
        "ci".to_string()
    } else {
        "release".to_string()
    };

    // Try to get git commit hash from environment or build
    let git_commit = std::env::var("GIT_COMMIT")
        .or_else(|_| std::env::var("CI_COMMIT_SHA"))
        .ok()
        .map(|s| s.chars().take(8).collect()); // Short hash

    Json(VersionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_type,
        git_commit,
    })
}

// ---------------------------------------------------------------------------
// GET /api/changelog
// ---------------------------------------------------------------------------

#[get("/api/changelog")]
pub fn changelog() -> Option<String> {
    std::fs::read_to_string("docs/CHANGELOG.md").ok()
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![system_info, desktop_health, version_info, changelog]
}
