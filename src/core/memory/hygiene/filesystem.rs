//! Filesystem-level memory hygiene: archiving and purging old daily
//! memory files and session logs based on age thresholds.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration as StdDuration, SystemTime};

use anyhow::{Context, Result};
use chrono::{Duration, Local, NaiveDate};

/// Move daily memory files older than `archive_after_days` into an
/// archive subdirectory.
///
/// # Errors
///
/// Returns an error if the archive directory cannot be created or
/// files cannot be moved.
pub(super) fn archive_daily_memory_files(
    workspace_dir: &Path,
    archive_after_days: u32,
) -> Result<u64> {
    if archive_after_days == 0 {
        return Ok(0);
    }

    let memory_dir = workspace_dir.join("memory");
    if !memory_dir.is_dir() {
        return Ok(0);
    }

    let archive_dir = memory_dir.join("archive");
    fs::create_dir_all(&archive_dir).context("create memory archive directory")?;

    let cutoff = Local::now().date_naive() - Duration::days(i64::from(archive_after_days));
    let mut moved = 0_u64;

    for entry in fs::read_dir(&memory_dir).context("read memory directory for archiving")? {
        let entry = entry.context("read memory directory entry")?;
        let path = entry.path();

        if path.is_dir() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };

        let Some(file_date) = memory_date_from_filename(filename) else {
            continue;
        };

        if file_date < cutoff {
            move_to_archive(&path, &archive_dir).context("archive daily memory file")?;
            moved += 1;
        }
    }

    Ok(moved)
}

/// Move session files older than `archive_after_days` into an archive
/// subdirectory.
///
/// # Errors
///
/// Returns an error if the archive directory cannot be created or
/// files cannot be moved.
pub(super) fn archive_session_files(workspace_dir: &Path, archive_after_days: u32) -> Result<u64> {
    if archive_after_days == 0 {
        return Ok(0);
    }

    let sessions_dir = workspace_dir.join("sessions");
    if !sessions_dir.is_dir() {
        return Ok(0);
    }

    let archive_dir = sessions_dir.join("archive");
    fs::create_dir_all(&archive_dir).context("create session archive directory")?;

    let cutoff_date = Local::now().date_naive() - Duration::days(i64::from(archive_after_days));
    let cutoff_time = SystemTime::now()
        .checked_sub(StdDuration::from_secs(
            u64::from(archive_after_days) * 24 * 60 * 60,
        ))
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut moved = 0_u64;
    for entry in fs::read_dir(&sessions_dir).context("read sessions directory for archiving")? {
        let entry = entry.context("read session directory entry")?;
        let path = entry.path();

        if path.is_dir() {
            continue;
        }

        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };

        let is_old = if let Some(date) = date_prefix(filename) {
            date < cutoff_date
        } else {
            is_older_than(&path, cutoff_time)
        };

        if is_old {
            move_to_archive(&path, &archive_dir).context("archive session file")?;
            moved += 1;
        }
    }

    Ok(moved)
}

/// Delete archived memory files older than `purge_after_days`.
///
/// # Errors
///
/// Returns an error if the archive directory cannot be read or files
/// cannot be removed.
pub(super) fn purge_memory_archives(workspace_dir: &Path, purge_after_days: u32) -> Result<u64> {
    if purge_after_days == 0 {
        return Ok(0);
    }

    let archive_dir = workspace_dir.join("memory").join("archive");
    if !archive_dir.is_dir() {
        return Ok(0);
    }

    let cutoff = Local::now().date_naive() - Duration::days(i64::from(purge_after_days));
    let mut removed = 0_u64;

    for entry in fs::read_dir(&archive_dir).context("read memory archive directory")? {
        let entry = entry.context("read memory archive entry")?;
        let path = entry.path();

        if path.is_dir() {
            continue;
        }

        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };

        let Some(file_date) = memory_date_from_filename(filename) else {
            continue;
        };

        if file_date < cutoff {
            fs::remove_file(&path).context("remove expired memory archive file")?;
            removed += 1;
        }
    }

    Ok(removed)
}

/// Delete archived session files older than `purge_after_days`.
///
/// # Errors
///
/// Returns an error if the archive directory cannot be read or files
/// cannot be removed.
pub(super) fn purge_session_archives(workspace_dir: &Path, purge_after_days: u32) -> Result<u64> {
    if purge_after_days == 0 {
        return Ok(0);
    }

    let archive_dir = workspace_dir.join("sessions").join("archive");
    if !archive_dir.is_dir() {
        return Ok(0);
    }

    let cutoff_date = Local::now().date_naive() - Duration::days(i64::from(purge_after_days));
    let cutoff_time = SystemTime::now()
        .checked_sub(StdDuration::from_secs(
            u64::from(purge_after_days) * 24 * 60 * 60,
        ))
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut removed = 0_u64;
    for entry in fs::read_dir(&archive_dir).context("read session archive directory")? {
        let entry = entry.context("read session archive entry")?;
        let path = entry.path();

        if path.is_dir() {
            continue;
        }

        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };

        let is_old = if let Some(date) = date_prefix(filename) {
            date < cutoff_date
        } else {
            is_older_than(&path, cutoff_time)
        };

        if is_old {
            fs::remove_file(&path).context("remove expired session archive file")?;
            removed += 1;
        }
    }

    Ok(removed)
}

fn memory_date_from_filename(filename: &str) -> Option<NaiveDate> {
    let stem = filename.strip_suffix(".md")?;
    let date_part = stem.split('_').next().unwrap_or(stem);
    NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok()
}

fn date_prefix(filename: &str) -> Option<NaiveDate> {
    if filename.len() < 10 {
        return None;
    }
    let boundary = crate::utils::text::floor_char_boundary(filename, 10);
    NaiveDate::parse_from_str(&filename[..boundary], "%Y-%m-%d").ok()
}

fn is_older_than(path: &Path, cutoff: SystemTime) -> bool {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .map(|modified| modified < cutoff)
        .unwrap_or(false)
}

fn move_to_archive(src: &Path, archive_dir: &Path) -> Result<()> {
    let Some(filename) = src.file_name().and_then(|f| f.to_str()) else {
        return Ok(());
    };

    let target = unique_archive_target(archive_dir, filename)?;
    fs::rename(src, target).context("move file to archive")?;
    Ok(())
}

fn unique_archive_target(archive_dir: &Path, filename: &str) -> Result<PathBuf> {
    use std::fs::OpenOptions;

    let direct = archive_dir.join(filename);
    // Atomic create: create_new(true) fails with AlreadyExists if the file
    // exists, eliminating the TOCTOU race between exists() and rename().
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&direct)
    {
        Ok(_) => return Ok(direct),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e).context("create archive target"),
    }

    let (stem, ext) = split_name(filename);
    for i in 1..10_000 {
        let candidate = if ext.is_empty() {
            archive_dir.join(format!("{stem}_{i}"))
        } else {
            archive_dir.join(format!("{stem}_{i}.{ext}"))
        };
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e).context("create archive target"),
        }
    }

    anyhow::bail!("failed to find unique archive name for '{filename}' after 10,000 attempts")
}

fn split_name(filename: &str) -> (&str, &str) {
    match filename.rsplit_once('.') {
        Some((stem, ext)) => (stem, ext),
        None => (filename, ""),
    }
}
