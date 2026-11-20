#![forbid(unsafe_code)]

//! File archive: an [`ArchiveStore`] that writes each retained item as one file
//! under a directory, its metadata in a sidecar in a tree beside it, and
//! restores it by reading both back.
//!
//! A xmip-core-archive **technology** (repository-model.md): it depends on the
//! archive capability for the [`ArchiveStore`] trait and its item, receipt and
//! error types, never the reverse. One item is `<root>/<data_type>/<identifier>`
//! holding the bytes verbatim — an operator opens it with whatever reads that
//! data type — and `<root>/.meta/<data_type>/<identifier>`, the TOML sidecar
//! holding the same four fields every archive technology keeps: `data_type`,
//! `identifier` and the `metadata` pairs, the bytes being the file itself, with
//! the SHA-256 of the bytes. The receipt carries that checksum, and `restore`
//! checks it. Each segment is a safe file name (`archive::layout::sanitize`).
//!
//! An archived item is never written over (ADR-0040, amendment 2026-11-19):
//! this crate is the [`Named`] store of a directory, and the rule — the
//! metadata first as the claim, then the bytes, a receipt only after both, an
//! identifier archived again answered with the same receipt or refused — is
//! the capability's `archive::named`, once for every name-keyed archive. A
//! file is created whole, synced, linked to its name only where the name is
//! free and its directory synced (`archive::whole`), so an item is on disk
//! before its receipt is given and a write that fails leaves nothing at its
//! name; a file a process that died left under its mark is removed by the
//! sweep the next write into its directory starts beside it, never one a
//! live process holds (ADR-0040, amendment 2026-11-20). Identifiers that
//! sanitize alike are refused in words. The receipt, `file:///<root>/<key>`,
//! is the capability's for every archive on a file system
//! (`archive::directory`), and only a key whose every segment names a file
//! under the root is read or written.

use std::fmt::Display;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use archive::directory::Directory;
use archive::layout::sanitize;
use archive::named::{self, Created, Named};
use archive::{ArchiveError, ArchiveItem, ArchiveReceipt, ArchiveStore, whole};

/// An archive that persists items as files rooted at a directory.
pub struct FileArchive {
    directory: Directory,
}

impl FileArchive {
    /// An archive writing under `root`; the directory tree is created on demand
    /// as items are archived.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            directory: Directory::new(root),
        }
    }
}

impl ArchiveStore for FileArchive {
    fn archive(&self, item: ArchiveItem) -> Result<ArchiveReceipt, ArchiveError> {
        named::archive(self, "", item)
    }

    fn restore(&self, receipt: &ArchiveReceipt) -> Result<ArchiveItem, ArchiveError> {
        named::restore(self, "", receipt)
    }
}

impl Named for FileArchive {
    fn create(&self, key: &str, bytes: &[u8]) -> Result<Created, ArchiveError> {
        let path = self.directory.path_of(key)?;
        whole::create(&path, |file| file.write_all(bytes)).map_err(|cause| at(&path, cause))
    }

    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, ArchiveError> {
        let path = self.directory.path_of(key)?;
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(missing) if missing.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(cause) => Err(at(&path, cause)),
        }
    }

    fn location(&self, key: &str) -> String {
        self.directory.location(key)
    }

    /// The key a receipt names, refusing a receipt from under another root:
    /// the key is the receipt's own segments, so nothing outside the root is
    /// ever read.
    fn key(&self, location: &str) -> Result<String, ArchiveError> {
        self.directory.key(location).map(str::to_string)
    }

    /// A safe file name: a directory holds no `/`, and no `..` survives.
    fn segment(&self, text: &str) -> String {
        sanitize(text)
    }
}

fn at(path: &Path, cause: impl Display) -> ArchiveError {
    ArchiveError {
        message: format!("{}: {cause}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archive::checksum::sha256_hex;
    use archive::sidecar::Sidecar;
    use file::temporary::Folder;

    fn item(id: &str) -> ArchiveItem {
        ArchiveItem {
            data_type: "json".to_string(),
            identifier: id.to_string(),
            bytes: b"{\"kept\":true}".to_vec(),
            metadata: vec![
                ("source".to_string(), "essentials".to_string()),
                (
                    "note".to_string(),
                    "with \"quotes\" and a\nnewline".to_string(),
                ),
            ],
        }
    }

    fn other_bytes(id: &str) -> ArchiveItem {
        ArchiveItem {
            bytes: b"{\"kept\":false}".to_vec(),
            ..item(id)
        }
    }

    /// A receipt for `relative` under the store's root, with no checksum.
    fn receipt_at(store: &FileArchive, relative: &str) -> ArchiveReceipt {
        ArchiveReceipt {
            location: store.location(relative),
            checksum: None,
        }
    }

    /// The names in `dir`, sorted; none where it is not there.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).map_or_default(|entries| {
            entries
                .map(|entry| {
                    entry
                        .expect("an entry")
                        .file_name()
                        .to_string_lossy()
                        .into()
                })
                .collect()
        });
        names.sort();
        names
    }

    #[test]
    fn an_archived_item_is_a_file_with_its_metadata_beside_it() {
        let root = Folder::new("archive-file-roundtrip");
        let store = FileArchive::new(&*root);
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
        let meta = root.join(".meta").join("json").join("json-1");
        let sidecar = std::fs::read_to_string(meta).expect("the sidecar");
        assert!(sidecar.starts_with("[item]\n"), "{sidecar}");
        assert!(
            sidecar.contains("[metadata]\n\"source\" = \"essentials\""),
            "{sidecar}"
        );
        assert_eq!(
            Sidecar::parse(&sidecar).expect("a sidecar").checksum,
            sha256_hex(&original.bytes)
        );
        let restored = store.restore(&receipt).expect("restore");
        assert_eq!(
            restored, original,
            "the file and its sidecar give the item back"
        );
    }

    #[test]
    fn an_identifier_archived_again_with_the_same_bytes_answers_the_same_receipt() {
        let root = Folder::new("archive-file-same");
        let store = FileArchive::new(&*root);
        let first = store.archive(item("json-1")).expect("archive");
        let again = store.archive(item("json-1")).expect("archived again");
        assert_eq!(again, first);
        assert_eq!(names(&root.join("json")), ["json-1"]);
    }

    #[test]
    fn an_identifier_archived_again_with_other_bytes_is_refused_and_the_earlier_receipt_restores() {
        let root = Folder::new("archive-file-other");
        let store = FileArchive::new(&*root);
        let receipt = store.archive(item("json-1")).expect("archive");
        let refused = store
            .archive(other_bytes("json-1"))
            .expect_err("never written over");
        assert!(
            refused
                .message
                .contains("is archived already with other bytes"),
            "{refused}"
        );
        assert_eq!(store.restore(&receipt).expect("restore"), item("json-1"));
        assert_eq!(names(&root.join("json")), ["json-1"]);
    }

    #[test]
    fn two_identifiers_that_sanitize_alike_are_refused() {
        let root = Folder::new("archive-file-alike");
        let store = FileArchive::new(&*root);
        let receipt = store.archive(item("json.1")).expect("archive");
        let refused = store
            .archive(item("json_1"))
            .expect_err("the same file name");
        assert!(
            refused
                .message
                .contains("is laid out at the same name and is refused"),
            "{refused}"
        );
        assert_eq!(store.restore(&receipt).expect("restore"), item("json.1"));
    }

    #[test]
    fn order_and_order_meta_are_two_items() {
        let root = Folder::new("archive-file-order");
        let store = FileArchive::new(&*root);
        let order = item("order");
        let order_meta = ArchiveItem {
            bytes: b"not order's metadata".to_vec(),
            ..item("order.meta")
        };
        let first = store.archive(order.clone()).expect("order");
        let second = store.archive(order_meta.clone()).expect("order.meta");
        assert_eq!(store.restore(&first).expect("order"), order);
        assert_eq!(store.restore(&second).expect("order.meta"), order_meta);
    }

    #[test]
    fn an_interrupted_pair_is_completed_by_its_repeat_and_refuses_another_item() {
        let root = Folder::new("archive-file-interrupted");
        let store = FileArchive::new(&*root);
        // An archive stopped after its claim: the metadata is there, the
        // bytes are not.
        let original = item("json-1");
        let claim = Sidecar {
            data_type: original.data_type.clone(),
            identifier: original.identifier.clone(),
            archived_at: "2026-11-19T00:00:00Z".to_string(),
            checksum: sha256_hex(&original.bytes),
            metadata: original.metadata.clone(),
        };
        let claimed = root.join(".meta").join("json");
        std::fs::create_dir_all(&claimed).expect("the metadata directory");
        std::fs::write(claimed.join("json-1"), claim.to_toml()).expect("the claim");
        let refused = store.archive(other_bytes("json-1")).expect_err("claimed");
        assert!(
            refused
                .message
                .contains("is archived already with other bytes"),
            "{refused}"
        );
        let receipt = store.archive(original.clone()).expect("completed");
        assert_eq!(store.restore(&receipt).expect("restore"), original);
        let refused = store.archive(other_bytes("json-1")).expect_err("taken");
        assert!(
            refused
                .message
                .contains("is archived already with other bytes"),
            "{refused}"
        );
    }

    #[test]
    fn bytes_without_metadata_are_refused_on_restore() {
        let root = Folder::new("archive-file-unclaimed");
        let store = FileArchive::new(&*root);
        std::fs::create_dir_all(root.join("json")).expect("the directory");
        std::fs::write(root.join("json").join("json-1"), b"{\"kept\":true}").expect("bytes");
        for relative in ["json/json-1", "json/never-archived"] {
            let refused = store
                .restore(&receipt_at(&store, relative))
                .expect_err(relative);
            assert!(
                refused
                    .message
                    .contains("is not an item this archive completed"),
                "{refused}"
            );
        }
    }

    #[test]
    fn a_foreign_object_at_the_key_is_refused() {
        let root = Folder::new("archive-file-foreign");
        let store = FileArchive::new(&*root);
        let path = root.join("json").join("json-1");
        std::fs::create_dir_all(root.join("json")).expect("the directory");
        std::fs::write(&path, b"someone else's").expect("a foreign file");
        let refused = store.archive(item("json-1")).expect_err("foreign bytes");
        assert!(
            refused
                .message
                .contains("holds bytes this archive did not write there"),
            "{refused}"
        );
        assert_eq!(std::fs::read(&path).expect("left"), b"someone else's");
        assert_eq!(names(&root.join("json")), ["json-1"]);
    }

    #[test]
    fn a_data_type_beginning_with_a_dot_is_refused() {
        let root = Folder::new("archive-file-dot");
        let store = FileArchive::new(&*root);
        let hidden = ArchiveItem {
            data_type: ".meta".to_string(),
            ..item("json-1")
        };
        let refused = store.archive(hidden).expect_err("a dot first");
        assert!(refused.message.contains("begins with a dot"), "{refused}");
        assert!(names(&root).is_empty(), "nothing is written");
    }

    #[test]
    fn a_write_failing_half_way_leaves_no_item() {
        let root = Folder::new("archive-file-half-way");
        let store = FileArchive::new(&*root);
        let original = item("json-1");
        // The archive's claim, and then its bytes failing half way.
        let claim = Sidecar {
            data_type: original.data_type.clone(),
            identifier: original.identifier.clone(),
            archived_at: "2026-11-19T00:00:00Z".to_string(),
            checksum: sha256_hex(&original.bytes),
            metadata: original.metadata.clone(),
        };
        let claimed = store.create(".meta/json/json-1", claim.to_toml().as_bytes());
        assert_eq!(claimed.expect("the claim"), Created::Created);
        let path = root.join("json").join("json-1");
        let failed = whole::create(&path, |file| {
            file.write_all(&original.bytes[..4])?;
            Err(io::Error::other("the disk is full"))
        });
        assert!(failed.is_err());
        assert!(names(&root.join("json")).is_empty(), "nothing at the name");
        let refused = store
            .restore(&receipt_at(&store, "json/json-1"))
            .expect_err("no item");
        assert!(
            refused
                .message
                .contains("is not an item this archive completed"),
            "{refused}"
        );
        let receipt = store.archive(original.clone()).expect("archived whole");
        assert_eq!(store.restore(&receipt).expect("restore"), original);
    }

    #[test]
    fn a_file_left_under_its_mark_is_removed_by_the_next_archive_into_its_directory() {
        let root = Folder::new("archive-file-left");
        let store = FileArchive::new(&*root);
        // What a process that died between a write and its link left: a
        // file under the mark, held by no process.
        let left = "json-0.xmip-archiving.4294967295.0";
        for directory in [root.join("json"), root.join(".meta").join("json")] {
            std::fs::create_dir_all(&directory).expect("the directory");
            std::fs::write(directory.join(left), b"half").expect("a file left behind");
        }
        let receipt = store.archive(item("json-1")).expect("archive");
        whole::swept(&root.join("json"));
        whole::swept(&root.join(".meta").join("json"));
        assert_eq!(names(&root.join("json")), ["json-1"]);
        assert_eq!(names(&root.join(".meta").join("json")), ["json-1"]);
        assert_eq!(store.restore(&receipt).expect("restore"), item("json-1"));
    }

    #[test]
    fn a_file_another_archive_is_writing_is_never_removed() {
        let root = Folder::new("archive-file-live");
        let store = FileArchive::new(&*root);
        std::fs::create_dir_all(root.join("json")).expect("the directory");
        // Another archive's write into the same directory, still going.
        let writing = root.join("json").join("json-0.xmip-archiving.4294967295.0");
        let live = file::held::create(&writing).expect("held by its writer");
        store.archive(item("json-1")).expect("archive");
        whole::swept(&root.join("json"));
        assert_eq!(
            names(&root.join("json")),
            ["json-0.xmip-archiving.4294967295.0", "json-1"]
        );
        drop(live);
    }

    #[test]
    fn the_receipt_carries_the_sha256_of_the_bytes() {
        let root = Folder::new("archive-file-checksum");
        let store = FileArchive::new(&*root);
        let receipt = store.archive(item("json-2")).expect("archive");
        let checksum = receipt.checksum.expect("a checksum");
        assert_eq!(checksum.len(), 64, "{checksum}");
        assert!(
            checksum.chars().all(|c| c.is_ascii_hexdigit()),
            "{checksum}"
        );
        assert_eq!(checksum, sha256_hex(b"{\"kept\":true}"));
    }

    #[test]
    fn a_risky_identifier_is_made_a_safe_file_name_and_kept_in_the_sidecar() {
        let root = Folder::new("archive-file-sanitize");
        let store = FileArchive::new(&*root);
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
    }

    #[test]
    fn a_mismatched_checksum_in_the_receipt_is_refused() {
        let root = Folder::new("archive-file-mismatch");
        let store = FileArchive::new(&*root);
        let mut receipt = store.archive(item("json-4")).expect("archive");
        receipt.checksum = Some("0".repeat(64));
        let refused = store.restore(&receipt).expect_err("the receipt lies");
        assert!(refused.message.contains("checksum mismatch"), "{refused}");
    }

    #[test]
    fn a_receipt_from_under_another_root_is_refused() {
        let root = Folder::new("archive-file-elsewhere");
        let store = FileArchive::new(&*root);
        let receipt = ArchiveReceipt {
            location: "file:///elsewhere/json/x".to_string(),
            checksum: None,
        };
        let refused = store.restore(&receipt).expect_err("another root");
        assert!(refused.message.contains("not a receipt"), "{refused}");
    }
}
