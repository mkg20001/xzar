mod api;
mod nix;
mod upload;

use std::io::{self, BufRead};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use tracing_subscriber::EnvFilter;

use crate::api::ApiClient;
use crate::nix::NixStore;
use crate::upload::UploadManager;

/// xzar - CLI client for xzar Nix binary cache
#[derive(Parser, Debug)]
#[command(name = "xzar", version, about)]
struct Args {
    /// Cache server URL
    #[arg(short, long)]
    server: String,

    /// API authentication key
    #[arg(short, long)]
    key: String,

    /// Pin name to create/update
    #[arg(short, long)]
    pin: String,

    /// Description for the pin
    #[arg(short, long)]
    desc: Option<String>,

    /// Use all CPU resources for parallel uploads
    #[arg(short, long, default_value = "true")]
    aggressive: bool,

    /// Auto-expiry duration for pin (e.g., "7d", "2w", "1m")
    #[arg(short, long)]
    expires: Option<String>,

    /// Duration to leave pin after replacement (e.g., "7d")
    #[arg(short, long)]
    leave_after_abandon: Option<String>,

    /// Nix store paths to upload
    #[arg(trailing_var_arg = true)]
    paths: Vec<PathBuf>,
}

fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Try parsing as plain number (milliseconds)
    if let Ok(ms) = s.parse::<u64>() {
        return Some(ms);
    }

    // Parse duration string like "7d", "2w", "1m", "1y"
    let (num_str, unit) = s.split_at(s.len().saturating_sub(1));
    let num: u64 = num_str.parse().ok()?;

    let multiplier = match unit {
        "d" => 24 * 60 * 60 * 1000,        // days
        "w" => 7 * 24 * 60 * 60 * 1000,    // weeks
        "m" => 30 * 24 * 60 * 60 * 1000,   // months (30 days)
        "y" => 365 * 24 * 60 * 60 * 1000,  // years
        _ => return None,
    };

    Some(num * multiplier)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    let args = Args::parse();

    // Collect paths from args and stdin
    let mut paths: Vec<PathBuf> = args.paths.clone();

    // Read from stdin if not a TTY
    if !atty::is(atty::Stream::Stdin) {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let line = line?;
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                paths.push(PathBuf::from(trimmed));
            }
        }
    }

    if paths.is_empty() {
        anyhow::bail!("No paths provided. Pass paths as arguments or via stdin.");
    }

    // Parse duration options
    let expires = args.expires.as_ref().and_then(|s| parse_duration(s));
    let leave_after_abandon = args.leave_after_abandon.as_ref().and_then(|s| parse_duration(s));

    // Determine parallelism
    let parallelism = if args.aggressive {
        num_cpus::get()
    } else {
        1
    };

    println!("Resolving Nix store closure...");

    // Get closure of all paths
    let nix = NixStore::new();
    let closure = nix.get_closure(&paths).await
        .context("Failed to get Nix store closure")?;

    println!("Found {} paths in closure", closure.len());

    // Create API client
    let api = ApiClient::new(&args.server, &args.key)?;

    // Check which paths need to be uploaded
    println!("Checking server for existing paths...");
    let need = api.check(&closure).await
        .context("Failed to check paths with server")?;

    if need.is_empty() {
        println!("All paths already in cache!");
    } else {
        println!("{} paths need to be uploaded", need.len());

        // Create progress bar
        let progress = ProgressBar::new(need.len() as u64);
        progress.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );

        // Create upload manager and run uploads
        let mut manager = UploadManager::new(
            api.clone(),
            nix,
            progress.clone(),
            parallelism,
            args.aggressive,
        );

        manager.upload_all(&need).await
            .context("Upload failed")?;

        progress.finish_with_message("Upload complete!");
    }

    // Finalize the pin
    println!("Finalizing pin '{}'...", args.pin);

    // Get root paths (basenames)
    let roots: Vec<String> = paths
        .iter()
        .filter_map(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .collect();

    api.finalize_pin(&args.pin, args.desc.as_deref(), &roots, expires, leave_after_abandon)
        .await
        .context("Failed to finalize pin")?;

    println!("Done! Pin '{}' created with {} roots", args.pin, roots.len());

    Ok(())
}

// Check if stdin is a TTY
mod atty {
    pub enum Stream {
        Stdin,
    }

    pub fn is(_stream: Stream) -> bool {
        unsafe { libc::isatty(libc::STDIN_FILENO) != 0 }
    }
}
