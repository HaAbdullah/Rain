//! The folder walker: reads one drive with a pool of threads and builds the tree in memory.
//! Section 05 6.3. Copied in shape from the proven spike `winapi/src/walk.rs`.
//!
//! The walk reads names, sizes and attributes only. It never opens a file, so a file that
//! another program has locked (`pagefile.sys`) is counted like any other.

use std::ffi::OsString;
use std::io;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

use rain_types::limits::SCAN_MAX_DEPTH;
use rayon::prelude::*;

use super::category::{CATEGORIES, category_of, index_of};

/// `FILE_ATTRIBUTE_DIRECTORY`.
const ATTR_DIRECTORY: u32 = 0x10;
/// `FILE_ATTRIBUTE_REPARSE_POINT`: a junction, a symbolic link, a mount point, a cloud
/// placeholder or an app execution alias.
const ATTR_REPARSE_POINT: u32 = 0x400;

/// Written in `DirNode::dominant` for a folder whose files hold no bytes at all.
pub const NO_CATEGORY: u8 = 255;

/// One entry of a folder as the file system lists it.
#[derive(Debug, Clone)]
pub struct RawEntry {
    pub name: OsString,
    /// Windows file attributes.
    pub attrs: u32,
    /// Logical size in bytes (0 for a folder).
    pub len: u64,
}

impl RawEntry {
    pub fn is_dir(&self) -> bool {
        self.attrs & ATTR_DIRECTORY != 0
    }

    pub fn is_reparse(&self) -> bool {
        self.attrs & ATTR_REPARSE_POINT != 0
    }
}

/// Where the walker reads folders from. The seam of the tests: a fake answers "access denied",
/// other errors and slow folders without touching a disk or its permissions.
pub trait DirSource: Send + Sync {
    fn list(&self, dir: &Path) -> io::Result<Vec<RawEntry>>;
}

/// The real file system.
pub struct StdDirs;

impl DirSource for StdDirs {
    fn list(&self, dir: &Path) -> io::Result<Vec<RawEntry>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            // An entry that vanished while the folder was read is not a problem of the folder.
            let Ok(entry) = entry else { continue };
            // On Windows this costs nothing: the data comes with the directory listing.
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            out.push(RawEntry {
                name: entry.file_name(),
                attrs: metadata.file_attributes(),
                len: metadata.len(),
            });
        }
        Ok(out)
    }
}

/// What the walker counted. Every field only grows; the threads add to them once per folder.
#[derive(Debug, Default)]
pub struct Counters {
    /// Files (not folders) seen.
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    /// Bytes of all files seen.
    pub bytes: AtomicU64,
    /// Folders that Windows did not let Rain read, plus folders cut off by the depth limit.
    pub denied: AtomicU64,
    /// Reparse points that were skipped.
    pub reparse: AtomicU64,
    /// Folders that vanished while the scan ran.
    pub vanished: AtomicU64,
}

/// A file that is kept as a node.
#[derive(Debug, Clone)]
pub struct FileNode {
    pub name: Box<str>,
    pub size: u64,
}

/// A folder and everything below it, as the walker builds it.
#[derive(Debug, Clone)]
pub struct DirNode {
    pub name: Box<str>,
    /// Bytes of every file below this folder, kept or not.
    pub size: u64,
    /// The largest files of this folder, at most `keep`.
    pub files: Vec<FileNode>,
    /// The files that were not kept: how many, and their bytes.
    pub other_files: (u64, u64),
    pub dirs: Vec<DirNode>,
    pub denied: bool,
    /// Index into `CATEGORIES` of the category with most bytes below this folder, or
    /// `NO_CATEGORY`.
    pub dominant: u8,
}

impl DirNode {
    fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            size: 0,
            files: Vec::new(),
            other_files: (0, 0),
            dirs: Vec::new(),
            denied: false,
            dominant: NO_CATEGORY,
        }
    }
}

/// What every call of `walk_dir` shares.
pub struct Ctx<'a> {
    pub src: &'a dyn DirSource,
    pub cancel: &'a AtomicBool,
    pub counters: &'a Counters,
    /// Files kept per folder.
    pub keep: usize,
}

/// Bytes per category of one subtree (index = `index_of(category)`).
pub type CatBytes = [u64; 8];

/// Walks the folder `path` and the folders below it. The folders of one level are walked in
/// parallel by rayon (work stealing), so this is called inside `ThreadPool::install`.
pub fn walk_dir(path: &Path, name: &str, depth: usize, ctx: &Ctx) -> (DirNode, CatBytes) {
    let mut node = DirNode::new(name);
    let mut cats: CatBytes = [0; 8];
    // Checked at the top of every folder: a cancel takes effect within milliseconds and what has
    // been built so far is still a valid (partial) tree.
    if ctx.cancel.load(Relaxed) {
        return (node, cats);
    }
    if depth >= SCAN_MAX_DEPTH {
        // A loop that the reparse check missed, or a pathological tree.
        ctx.counters.denied.fetch_add(1, Relaxed);
        node.denied = true;
        return (node, cats);
    }
    let entries = match ctx.src.list(path) {
        Ok(entries) => entries,
        Err(error) => {
            // "Not accessible" is a folder that exists but cannot be read, for whatever reason.
            // One that disappeared while we scanned (a temp folder) is simply empty.
            if error.kind() == io::ErrorKind::NotFound {
                ctx.counters.vanished.fetch_add(1, Relaxed);
            } else {
                ctx.counters.denied.fetch_add(1, Relaxed);
                node.denied = true;
            }
            return (node, cats);
        }
    };

    let mut files: Vec<FileNode> = Vec::new();
    let mut subfolders: Vec<(PathBuf, String)> = Vec::new();
    let mut folder_bytes = 0u64;
    for entry in entries {
        // Never followed and never counted: no loops, no double counting, and a cloud placeholder
        // is never opened (that would download it).
        if entry.is_reparse() {
            ctx.counters.reparse.fetch_add(1, Relaxed);
            continue;
        }
        let entry_name = entry.name.to_string_lossy().into_owned();
        if entry.is_dir() {
            subfolders.push((path.join(&entry.name), entry_name));
        } else {
            // All files count for the category bytes, also the ones that are not kept below.
            cats[index_of(category_of(&entry_name))] += entry.len;
            folder_bytes += entry.len;
            files.push(FileNode {
                name: entry_name.into_boxed_str(),
                size: entry.len,
            });
        }
    }

    ctx.counters.dirs.fetch_add(1, Relaxed);
    ctx.counters.files.fetch_add(files.len() as u64, Relaxed);
    ctx.counters.bytes.fetch_add(folder_bytes, Relaxed);

    // Keep the largest files only; the rest become one counted lump. Ties go by name, so the
    // result is the same on every run.
    if files.len() > ctx.keep {
        files.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
        let rest = files.split_off(ctx.keep);
        node.other_files = (rest.len() as u64, rest.iter().map(|file| file.size).sum());
    }
    node.size = folder_bytes;
    node.files = files;

    let results: Vec<(DirNode, CatBytes)> = subfolders
        .par_iter()
        .map(|(sub_path, sub_name)| walk_dir(sub_path, sub_name, depth + 1, ctx))
        .collect();
    for (child, child_cats) in results {
        node.size += child.size;
        for (total, part) in cats.iter_mut().zip(child_cats) {
            *total += part;
        }
        node.dirs.push(child);
    }
    node.dominant = dominant(&cats);
    (node, cats)
}

/// The index of the category with most bytes (the first one on a tie), or `NO_CATEGORY`.
fn dominant(cats: &CatBytes) -> u8 {
    let mut best: Option<usize> = None;
    for (index, bytes) in cats.iter().enumerate() {
        if *bytes > 0 && best.is_none_or(|b| *bytes > cats[b]) {
            best = Some(index);
        }
    }
    best.map_or(NO_CATEGORY, |index| index as u8)
}

/// The category stored in a `dominant` byte.
pub fn category_from_index(index: u8) -> Option<rain_types::FileCategory> {
    CATEGORIES.get(usize::from(index)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    /// A folder tree in memory. A folder with an error kind answers that error.
    #[derive(Default)]
    struct MemDirs {
        dirs: HashMap<PathBuf, Result<Vec<RawEntry>, io::ErrorKind>>,
        calls: AtomicUsize,
        /// Set the cancel flag when this many folders have been listed.
        cancel_after: Mutex<Option<(usize, std::sync::Arc<AtomicBool>)>>,
    }

    impl MemDirs {
        fn dir(mut self, path: &str, entries: Vec<RawEntry>) -> Self {
            self.dirs.insert(PathBuf::from(path), Ok(entries));
            self
        }

        fn failing(mut self, path: &str, kind: io::ErrorKind) -> Self {
            self.dirs.insert(PathBuf::from(path), Err(kind));
            self
        }
    }

    impl DirSource for MemDirs {
        fn list(&self, dir: &Path) -> io::Result<Vec<RawEntry>> {
            let calls = self.calls.fetch_add(1, Relaxed) + 1;
            if let Some((limit, flag)) = &*self.cancel_after.lock().unwrap()
                && calls >= *limit
            {
                flag.store(true, Relaxed);
            }
            match self.dirs.get(dir) {
                Some(Ok(entries)) => Ok(entries.clone()),
                Some(Err(kind)) => Err(io::Error::from(*kind)),
                None => Err(io::Error::from(io::ErrorKind::NotFound)),
            }
        }
    }

    fn file(name: &str, len: u64) -> RawEntry {
        RawEntry {
            name: name.into(),
            attrs: 0x20,
            len,
        }
    }

    fn folder(name: &str) -> RawEntry {
        RawEntry {
            name: name.into(),
            attrs: ATTR_DIRECTORY,
            len: 0,
        }
    }

    fn walk(src: &dyn DirSource, keep: usize) -> (DirNode, CatBytes, Counters) {
        let cancel = AtomicBool::new(false);
        walk_with(src, keep, &cancel)
    }

    fn walk_with(
        src: &dyn DirSource,
        keep: usize,
        cancel: &AtomicBool,
    ) -> (DirNode, CatBytes, Counters) {
        let counters = Counters::default();
        let ctx = Ctx {
            src,
            cancel,
            counters: &counters,
            keep,
        };
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap();
        let (node, cats) = pool.install(|| walk_dir(Path::new("T:\\"), "T:", 0, &ctx));
        (node, cats, counters)
    }

    #[test]
    fn sizes_add_up_and_the_counters_count_files_not_folders() {
        let src = MemDirs::default()
            .dir(
                "T:\\",
                vec![file("big.bin", 1000), folder("a"), folder("empty")],
            )
            .dir("T:\\a", vec![file("x.txt", 10), file("y.txt", 20)])
            .dir("T:\\empty", vec![]);
        let (tree, _, counters) = walk(&src, 64);
        assert_eq!(tree.size, 1030);
        let a = tree.dirs.iter().find(|d| &*d.name == "a").unwrap();
        assert_eq!(a.size, 30);
        assert_eq!(counters.files.load(Relaxed), 3);
        assert_eq!(counters.dirs.load(Relaxed), 3);
        assert_eq!(counters.bytes.load(Relaxed), 1030);
        assert_eq!(counters.denied.load(Relaxed), 0);
    }

    #[test]
    fn a_reparse_entry_is_skipped_and_counted_and_never_followed() {
        let link = RawEntry {
            name: "link".into(),
            attrs: ATTR_DIRECTORY | ATTR_REPARSE_POINT,
            len: 0,
        };
        // "T:\link" is not in the map: following it would be an error that shows up as `denied`.
        let src = MemDirs::default().dir("T:\\", vec![file("a", 5), link]);
        let (tree, _, counters) = walk(&src, 64);
        assert_eq!(tree.size, 5);
        assert!(tree.dirs.is_empty());
        assert_eq!(counters.reparse.load(Relaxed), 1);
        assert_eq!(counters.denied.load(Relaxed), 0);
    }

    #[test]
    fn a_folder_that_is_refused_is_denied_with_size_zero() {
        let src = MemDirs::default()
            .dir("T:\\", vec![file("a", 5), folder("locked")])
            .failing("T:\\locked", io::ErrorKind::PermissionDenied);
        let (tree, _, counters) = walk(&src, 64);
        let locked = &tree.dirs[0];
        assert!(locked.denied);
        assert_eq!(locked.size, 0);
        assert_eq!(counters.denied.load(Relaxed), 1);
    }

    #[test]
    fn a_folder_that_vanished_is_empty_not_denied_and_other_errors_are_denied() {
        let src = MemDirs::default()
            .dir("T:\\", vec![folder("gone"), folder("broken")])
            .failing("T:\\gone", io::ErrorKind::NotFound)
            .failing("T:\\broken", io::ErrorKind::InvalidData);
        let (tree, _, counters) = walk(&src, 64);
        let by_name = |name: &str| tree.dirs.iter().find(|d| &*d.name == name).unwrap();
        assert!(!by_name("gone").denied);
        assert!(by_name("broken").denied);
        assert_eq!(counters.denied.load(Relaxed), 1);
        assert_eq!(counters.vanished.load(Relaxed), 1);
    }

    #[test]
    fn a_chain_deeper_than_the_limit_ends_in_a_denied_node() {
        let mut src = MemDirs::default();
        let mut path = PathBuf::from("T:\\");
        for _ in 0..300 {
            src.dirs.insert(path.clone(), Ok(vec![folder("d")]));
            path = path.join("d");
        }
        let (tree, _, counters) = walk(&src, 64);
        assert_eq!(counters.denied.load(Relaxed), 1);
        let mut depth = 0;
        let mut node = &tree;
        while let Some(next) = node.dirs.first() {
            depth += 1;
            node = next;
        }
        assert_eq!(depth, SCAN_MAX_DEPTH);
        assert!(node.denied);
    }

    #[test]
    fn only_the_largest_files_are_kept_and_the_rest_is_counted() {
        let entries: Vec<RawEntry> = (0..70).map(|n| file(&format!("f{n:02}"), 10)).collect();
        let src = MemDirs::default().dir("T:\\", entries);
        let (tree, _, counters) = walk(&src, 64);
        assert_eq!(tree.files.len(), 64);
        assert_eq!(tree.other_files, (6, 60));
        assert_eq!(tree.size, 700, "the folded files are inside the size");
        assert_eq!(counters.files.load(Relaxed), 70);
    }

    #[test]
    fn the_dominant_category_counts_the_files_that_were_folded_too() {
        // Two big videos and sixty-six tiny documents: with keep = 2 the documents are folded,
        // yet together they hold more bytes than the one image.
        let mut entries = vec![file("a.png", 100), file("b.mp4", 50)];
        entries.extend((0..66).map(|n| file(&format!("d{n}.txt"), 10)));
        let src = MemDirs::default().dir("T:\\", entries);
        let (tree, cats, _) = walk(&src, 2);
        assert_eq!(cats[index_of(rain_types::FileCategory::Document)], 660);
        assert_eq!(
            category_from_index(tree.dominant),
            Some(rain_types::FileCategory::Document)
        );
    }

    #[test]
    fn a_folder_without_bytes_has_no_dominant_category() {
        let src = MemDirs::default().dir("T:\\", vec![file("empty.txt", 0)]);
        let (tree, _, _) = walk(&src, 64);
        assert_eq!(tree.dominant, NO_CATEGORY);
        assert_eq!(category_from_index(NO_CATEGORY), None);
    }

    #[test]
    fn a_cancel_stops_the_walk_and_leaves_a_valid_partial_tree() {
        let mut src =
            MemDirs::default().dir("T:\\", (0..20).map(|n| folder(&format!("d{n}"))).collect());
        for n in 0..20 {
            src = src.dir(&format!("T:\\d{n}"), vec![file("x", 1)]);
        }
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        *src.cancel_after.lock().unwrap() = Some((3, std::sync::Arc::clone(&cancel)));
        let (tree, _, counters) = walk_with(&src, 64, &cancel);
        assert!(src.calls.load(Relaxed) < 21, "not every folder was listed");
        assert!(tree.size <= 20);
        assert_eq!(counters.files.load(Relaxed), tree.size);
    }

    #[test]
    fn the_real_file_system_lists_names_sizes_and_attributes() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.txt"), b"12345").unwrap();
        std::fs::create_dir(temp.path().join("sub")).unwrap();
        let mut entries = StdDirs.list(temp.path()).unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].len, 5);
        assert!(!entries[0].is_dir());
        assert!(entries[1].is_dir());
        assert!(!entries[1].is_reparse());
        assert!(StdDirs.list(&temp.path().join("missing")).is_err());
    }
}
