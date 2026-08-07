use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

const SUPPORTED_EXTENSIONS: &[&str] = &["mp3", "wav", "flac", "ogg", "m4a", "aac"];

/// Get the available files in a directory.
/// If `path` is empty, returns a list of system drives (Virtual Root).
pub fn get_directory_content(path: &Path) -> io::Result<Vec<FileEntry>> {
    // Handle "Virtual Root" (List System Drives)
    if path.as_os_str().is_empty() {
        return Ok(get_system_drives());
    }

    let mut entries = Vec::new();

    if let Ok(read_dir) = fs::read_dir(path) {
        for entry in read_dir.flatten() {
            let entry_path = entry.path();
            let is_dir = entry_path.is_dir();

            // Filter: Include directories and supported audio files
            let should_include = is_dir
                || entry_path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
                    .unwrap_or(false);

            if should_include {
                let name = entry_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("???")
                    .to_string();

                entries.push(FileEntry {
                    name,
                    path: entry_path,
                    is_dir,
                });
            }
        }
    }

    // Sort: Directories first, then alphabetical
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    if let Some(parent) = path.parent() {
        entries.insert(
            0,
            FileEntry {
                name: "..".to_string(),
                path: parent.to_path_buf(),
                is_dir: true,
            },
        );
    } else {
        entries.insert(
            0,
            FileEntry {
                name: "..".to_string(),
                path: PathBuf::from(""),
                is_dir: true,
            },
        );
    }

    Ok(entries)
}

/// Helper to list available drives on Windows or Root on Unix
fn get_system_drives() -> Vec<FileEntry> {
    let mut drives = Vec::new();

    #[cfg(target_os = "windows")]
    {
        // Iterate A..Z to find available drives
        for c in b'A'..=b'Z' {
            let drive_letter = c as char;
            let root_str = format!("{}:\\", drive_letter);
            let root_path = PathBuf::from(&root_str);

            // Check if drive exists (this handles C:\, D:\, etc.)
            if root_path.exists() {
                drives.push(FileEntry {
                    name: root_str,
                    path: root_path,
                    is_dir: true,
                });
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // On Unix/Linux/Mac, the root is just "/"
        drives.push(FileEntry {
            name: "/".to_string(),
            path: PathBuf::from("/"),
            is_dir: true,
        });
    }

    drives
}
