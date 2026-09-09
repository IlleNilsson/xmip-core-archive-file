#![forbid(unsafe_code)]

//! File archive: an [`ArchiveStore`] that writes each retained item as one file
//! under a directory, its metadata in a sidecar beside it, and restores it by
//! reading both back.
//!
//! A xmip-core-archive **technology** (repository-model.md): it depends on the
//! archive capability for the [`ArchiveStore`] trait and its item, receipt and
//! error types, never the reverse. One item is `<root>/<data_type>/<identifier>`
//! holding the bytes verbatim — an operator opens it with whatever reads that
//! data type — and `<root>/<data_type>/<identifier>.meta` beside it, TOML
//! holding the same four fields every archive technology keeps: `data_type`,
//! `identifier` and the `metadata` pairs, the bytes being the file itself. The
//! receipt carries the SHA-256 of the bytes, and `restore` checks it.

mod clock;
mod meta;

use std::fmt::{Display, Write};
use std::path::{Path, PathBuf};

use archive::{ArchiveError, ArchiveItem, ArchiveReceipt, ArchiveStore};
use sha2::{Digest, Sha256};

use crate::meta::Meta;

/// An archive that persists items as files rooted at a directory.
pub struct FileArchive {
    root: PathBuf,
}

impl FileArchive {
    /// An archive writing under `root`; the directory tree is created on demand
    /// as items are archived.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// What every receipt of this root shares: `file:///<root>/`.
    fn prefix(&self) -> String {
        format!("file://{}/", uri_path(&self.root))
    }

    /// The item file a receipt names, refusing a receipt from under another
    /// root: the path is rebuilt from the receipt's own segments, so nothing
    /// outside the root is ever read.
    fn path_of(&self, location: &str) -> Result<PathBuf, ArchiveError> {
        let relative = location
            .strip_prefix(&self.prefix())
            .ok_or_else(|| ArchiveError {
                message: format!(
                    "{location} is not a receipt of the archive at {}",
                    self.root.display()
                ),
            })?;
        let mut path = self.root.clone();
        for segment in relative.split('/') {
            if segment.is_empty() || segment == "." || segment == ".." {
                return Err(ArchiveError {
                    message: format!("{location} does not name an item under the archive"),
                });
            }
            path.push(segment);
        }
        Ok(path)
    }
}

impl ArchiveStore for FileArchive {
    fn archive(&self, item: ArchiveItem) -> Result<ArchiveReceipt, ArchiveError> {
        let data_type = sanitise(&item.data_type);
        let identifier = sanitise(&item.identifier);
        let path = self.root.join(&data_type).join(&identifier);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(error)?;
        }
        std::fs::write(&path, &item.bytes).map_err(|cause| at(&path, cause))?;
        let sidecar = Meta {
            data_type: item.data_type,
            identifier: item.identifier,
            archived_at: clock::now(),
            metadata: item.metadata,
        };
        let meta_path = meta_path(&path);
        std::fs::write(&meta_path, sidecar.to_toml()).map_err(|cause| at(&meta_path, cause))?;
        Ok(ArchiveReceipt {
            location: format!("{}{data_type}/{identifier}", self.prefix()),
            checksum: Some(sha256(&item.bytes)),
        })
    }

    fn restore(&self, receipt: &ArchiveReceipt) -> Result<ArchiveItem, ArchiveError> {
        let path = self.path_of(&receipt.location)?;
        let bytes = std::fs::read(&path).map_err(|cause| at(&path, cause))?;
        if let Some(expected) = &receipt.checksum {
            let actual = sha256(&bytes);
            if actual != *expected {
                return Err(ArchiveError {
                    message: format!(
                        "checksum mismatch at {}: the receipt says {expected}, the file {actual}",
                        path.display()
                    ),
                });
            }
        }
        let meta_path = meta_path(&path);
        let text = std::fs::read_to_string(&meta_path).map_err(|cause| at(&meta_path, cause))?;
        let sidecar = Meta::parse(&text).map_err(|reason| at(&meta_path, reason))?;
        Ok(ArchiveItem {
            data_type: sidecar.data_type,
            identifier: sidecar.identifier,
            bytes,
            metadata: sidecar.metadata,
        })
    }
}

/// The sidecar beside an item file: the same name with `.meta` appended.
fn meta_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".meta");
    PathBuf::from(name)
}

/// The SHA-256 of `bytes` as lowercase hex, the receipt's checksum.
fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Make one path segment safe: anything but a plain filename character becomes an
/// underscore, so an identifier like `poison-json#3` is a valid file name.
fn sanitise(segment: &str) -> String {
    segment
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `path` as the path part of a URI: forward slashes, and a leading slash so a
/// Windows drive reads `/C:/...` after the `file://` authority.
fn uri_path(path: &Path) -> String {
    let text = path.display().to_string().replace('\\', "/");
    if text.starts_with('/') {
        text
    } else {
        format!("/{text}")
    }
}

fn at(path: &Path, cause: impl Display) -> ArchiveError {
    ArchiveError {
        message: format!("{}: {cause}", path.display()),
    }
}

fn error(cause: impl Display) -> ArchiveError {
    ArchiveError {
        message: cause.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xmip-file-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    fn item(id: &str) -> ArchiveItem {
        ArchiveItem {
            data_type: "json".to_string(),
            identifier: id.to_string(),
            bytes: b"{\"kept\":true}".to_vec(),
            metadata: vec![
                ("source".to_string(), "playground".to_string()),
                (
                    "note".to_string(),
                    "with \"quotes\" and a\nnewline".to_string(),
                ),
            ],
        }
    }

    #[test]
    fn an_archived_item_is_a_file_with_its_metadata_beside_it() {
        let root = scratch("roundtrip");
        let store = FileArchive::new(&root);
        let original = item("json-1");
        let receipt = store.archive(original.clone()).expect("archive");
        assert!(
            receipt.location.starts_with("file:///"),
            "{}",
            receipt.location
        );
        assert!(
            receipt.location.ends_with("/json/json-1"),
            "{}",
            receipt.location
        );
        let path = root.join("json").join("json-1");
        assert_eq!(std::fs::read(&path).expect("the bytes"), original.bytes);
        let sidecar = std::fs::read_to_string(meta_path(&path)).expect("the sidecar");
        assert!(sidecar.starts_with("[item]\n"), "{sidecar}");
        assert!(
            sidecar.contains("[metadata]\n\"source\" = \"playground\""),
            "{sidecar}"
        );
        let restored = store.restore(&receipt).expect("restore");
        assert_eq!(
            restored, original,
            "the file and its sidecar give the item back"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_receipt_carries_the_sha256_of_the_bytes() {
        let root = scratch("checksum");
        let store = FileArchive::new(&root);
        let receipt = store.archive(item("json-2")).expect("archive");
        let checksum = receipt.checksum.expect("a checksum");
        assert_eq!(checksum.len(), 64, "{checksum}");
        assert!(
            checksum.chars().all(|c| c.is_ascii_hexdigit()),
            "{checksum}"
        );
        assert_eq!(checksum, sha256(b"{\"kept\":true}"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_risky_identifier_is_made_a_safe_file_name_and_kept_in_the_sidecar() {
        let root = scratch("sanitise");
        let store = FileArchive::new(&root);
        let original = item("poison-json#3/../x");
        let receipt = store.archive(original.clone()).expect("archive");
        assert!(!receipt.location.contains(".."), "no traversal survives");
        assert!(
            receipt.location.ends_with("/json/poison-json_3____x"),
            "{}",
            receipt.location
        );
        let restored = store.restore(&receipt).expect("restore");
        assert_eq!(
            restored.identifier, "poison-json#3/../x",
            "the sidecar keeps the original"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_mismatched_checksum_in_the_receipt_is_refused() {
        let root = scratch("mismatch");
        let store = FileArchive::new(&root);
        let mut receipt = store.archive(item("json-4")).expect("archive");
        receipt.checksum = Some("0".repeat(64));
        let refused = store.restore(&receipt).expect_err("the receipt lies");
        assert!(refused.message.contains("checksum mismatch"), "{refused}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_missing_file_is_an_error_naming_the_path() {
        let root = scratch("missing");
        let store = FileArchive::new(&root);
        let receipt = ArchiveReceipt {
            location: format!("{}json/never-archived", store.prefix()),
            checksum: None,
        };
        let refused = store.restore(&receipt).expect_err("nothing there");
        let path = root.join("json").join("never-archived");
        assert!(
            refused.message.contains(&path.display().to_string()),
            "{refused}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_receipt_from_under_another_root_is_refused() {
        let root = scratch("other");
        let store = FileArchive::new(&root);
        let receipt = ArchiveReceipt {
            location: "file:///elsewhere/json/x".to_string(),
            checksum: None,
        };
        let refused = store.restore(&receipt).expect_err("another root");
        assert!(refused.message.contains("not a receipt"), "{refused}");
        std::fs::remove_dir_all(&root).ok();
    }
}
