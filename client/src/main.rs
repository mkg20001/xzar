mod api;
mod nix;
mod upload;

use std::io::{self, BufRead};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
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
    #[arg(short, long, global = true)]
    server: Option<String>,

    /// API authentication key
    #[arg(short, long, global = true)]
    key: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Upload paths to the cache and create a pin
    Upload {
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
    },

    /// List all pins on the server
    List,
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

    let server = args.server.ok_or_else(|| anyhow::anyhow!("--server is required"))?;
    let key = args.key.ok_or_else(|| anyhow::anyhow!("--key is required"))?;

    // Create API client
    let api = ApiClient::new(&server, &key)?;

    match args.command {
        Command::Upload {
            pin,
            desc,
            aggressive,
            expires,
            leave_after_abandon,
            paths,
        } => {
            cmd_upload(api, pin, desc, aggressive, expires, leave_after_abandon, paths).await
        }
        Command::List => cmd_list(api).await,
    }
}

async fn cmd_upload(
    api: ApiClient,
    pin: String,
    desc: Option<String>,
    aggressive: bool,
    expires: Option<String>,
    leave_after_abandon: Option<String>,
    mut paths: Vec<PathBuf>,
) -> Result<()> {
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
    let expires = expires.as_ref().and_then(|s| parse_duration(s));
    let leave_after_abandon = leave_after_abandon.as_ref().and_then(|s| parse_duration(s));

    // Determine parallelism
    let parallelism = if aggressive {
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
            aggressive,
        );

        manager.upload_all(&need).await
            .context("Upload failed")?;

        progress.finish_with_message("Upload complete!");
    }

    // Finalize the pin
    println!("Finalizing pin '{}'...", pin);

    // Get root paths (basenames) - resolve symlinks to get actual store paths
    let roots: Vec<String> = paths
        .iter()
        .filter_map(|p| {
            // Resolve symlink if it is one, otherwise use the path as-is
            let resolved = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            resolved.file_name().map(|s| s.to_string_lossy().to_string())
        })
        .collect();

    api.finalize_pin(&pin, desc.as_deref(), &roots, expires, leave_after_abandon)
        .await
        .context("Failed to finalize pin")?;

    println!("Done! Pin '{}' created with {} roots", pin, roots.len());

    Ok(())
}

async fn cmd_list(api: ApiClient) -> Result<()> {
    let pins = api.list_pins().await
        .context("Failed to list pins")?;

    if pins.is_empty() {
        println!("No pins found.");
        return Ok(());
    }

    for pin in pins {
        let status = if pin.abandoned {
            "abandoned"
        } else if pin.expires.is_some() {
            "expiring"
        } else {
            "active"
        };

        println!("{} ({}) - {} roots [{}]", pin.name, pin.id, pin.roots.len(), status);

        if let Some(desc) = &pin.description {
            println!("  Description: {}", desc);
        }

        println!("  Created: {}", pin.created);

        if let Some(expires) = &pin.expires {
            println!("  Expires: {}", expires);
        }

        for root in &pin.roots {
            println!("    /nix/store/{}", root.drv_full);
        }

        println!();
    }

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
