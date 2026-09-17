//! Reads and writes SigilWard's exact on-disk `Baseline` shape
//! (`~/.local/state/sigilward/baseline.json`) — same `FileRecord` fields,
//! same SHA-256/symlink-handling rules as `sigilward/src/baseline.rs`, so a
//! record Argus writes here is indistinguishable from one SigilWard itself
//! wrote. This is the real integration point: `sigilward update` and
//! Argus's own "accept" action both write into the same file.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FileRecord {
    pub sha256: String,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Baseline {
    pub files: BTreeMap<String, FileRecord>,
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Same rule as SigilWard: symlinks are recorded by their own metadata, not
/// followed — a repointed symlink is itself a change worth catching.
pub fn record_for(path: &Path) -> std::io::Result<FileRecord> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Ok(FileRecord {
            sha256: "symlink".to_string(),
            mode: meta.mode(),
            uid: meta.uid(),
            gid: meta.gid(),
            size: 0,
        });
    }
    Ok(FileRecord {
        sha256: hash_file(path)?,
        mode: meta.mode(),
        uid: meta.uid(),
        gid: meta.gid(),
        size: meta.len(),
    })
}

pub fn load(path: &Path) -> std::io::Result<Baseline> {
    let raw = std::fs::read_to_string(path)?;
    serde_json::from_str(&raw).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub fn save(baseline: &Baseline, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(baseline)?;
    std::fs::write(path, json)
}

/// Re-hashes `path` fresh and writes/updates its record directly in the
/// on-disk baseline at `baseline_path` — the "accept this change" action.
/// Read-modify-write against the file each call (not a long-held in-memory
/// copy) so a concurrent `sigilward update` isn't clobbered by a stale
/// write.
pub fn accept(baseline_path: &Path, target: &str) -> anyhow::Result<()> {
    let mut baseline = load(baseline_path).unwrap_or_default();
    let path = Path::new(target);
    if path.exists() || std::fs::symlink_metadata(path).is_ok() {
        let record = record_for(path)?;
        baseline.files.insert(target.to_string(), record);
    } else {
        // Deleted since — accepting a deletion means it's no longer part
        // of the trusted set.
        baseline.files.remove(target);
    }
    save(&baseline, baseline_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_for_hashes_consistently() {
        let dir = std::env::temp_dir().join(format!("argus-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        std::fs::write(&path, b"hello world").unwrap();

        let r1 = record_for(&path).unwrap();
        let r2 = record_for(&path).unwrap();
        assert_eq!(r1.sha256, r2.sha256);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_and_load_round_trip_matches_sigilwards_json_shape() {
        let dir = std::env::temp_dir().join(format!("argus-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("baseline.json");

        let mut baseline = Baseline::default();
        baseline.files.insert(
            "/etc/foo".to_string(),
            FileRecord {
                sha256: "abc".to_string(),
                mode: 0o644,
                uid: 0,
                gid: 0,
                size: 10,
            },
        );
        save(&baseline, &path).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"sha256\""));
        assert!(raw.contains("/etc/foo"));

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.files["/etc/foo"], baseline.files["/etc/foo"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn accept_inserts_a_fresh_record_for_a_new_path() {
        let dir = std::env::temp_dir().join(format!("argus-test3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let baseline_path = dir.join("baseline.json");
        let target = dir.join("watched.txt");
        std::fs::write(&target, b"reviewed and accepted").unwrap();

        save(&Baseline::default(), &baseline_path).unwrap();
        accept(&baseline_path, target.to_str().unwrap()).unwrap();

        let loaded = load(&baseline_path).unwrap();
        assert!(loaded.files.contains_key(target.to_str().unwrap()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn accept_removes_the_record_for_a_since_deleted_path() {
        let dir = std::env::temp_dir().join(format!("argus-test4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let baseline_path = dir.join("baseline.json");
        let gone = dir.join("gone.txt");

        let mut baseline = Baseline::default();
        baseline.files.insert(
            gone.to_str().unwrap().to_string(),
            FileRecord {
                sha256: "abc".to_string(),
                mode: 0o644,
                uid: 0,
                gid: 0,
                size: 10,
            },
        );
        save(&baseline, &baseline_path).unwrap();

        accept(&baseline_path, gone.to_str().unwrap()).unwrap();

        let loaded = load(&baseline_path).unwrap();
        assert!(!loaded.files.contains_key(gone.to_str().unwrap()));

        std::fs::remove_dir_all(&dir).ok();
    }
}
