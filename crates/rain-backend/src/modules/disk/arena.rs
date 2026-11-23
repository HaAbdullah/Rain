//! The finished scan in memory (the "arena") and the small tree that the screen gets from it
//! (the "budgeted tree"). Section 05 6.5.
//!
//! A drive can hold millions of files, so no answer carries the whole tree. `cut` picks the
//! biggest folders and opens them one after the other until a budget of nodes is used; what is
//! not listed is summed up in one `other` node per folder, so that the sizes stay exact.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use rain_types::limits::{SCAN_OPEN_FOLDER_CHILDREN, SCAN_PER_FOLDER, SCAN_TREE_BUDGET};
use rain_types::{DiskNode, FileCategory, NodeKind};

use super::category::category_of;
use super::walk::{DirNode, FileNode, category_from_index};

/// One folder of a finished scan.
#[derive(Debug)]
pub struct DirRec {
    pub name: Box<str>,
    pub size: u64,
    pub files: Vec<FileNode>,
    pub other_files: (u64, u64),
    /// Indexes into `Arena::dirs`.
    pub children: Vec<u32>,
    pub denied: bool,
    pub dominant: u8,
}

/// All folders of one scan in one vector. The root is index 0. About 150 bytes a folder, so a
/// drive with a million files fits in 60 to 200 MB.
#[derive(Debug)]
pub struct Arena {
    pub dirs: Vec<DirRec>,
}

/// Moves the tree that the walker built into the arena (no copy of the names).
pub fn flatten(tree: DirNode) -> Arena {
    let mut dirs = Vec::new();
    push(&mut dirs, tree);
    Arena { dirs }
}

fn push(dirs: &mut Vec<DirRec>, node: DirNode) -> u32 {
    let id = dirs.len() as u32;
    dirs.push(DirRec {
        name: node.name,
        size: node.size,
        files: node.files,
        other_files: node.other_files,
        children: Vec::new(),
        denied: node.denied,
        dominant: node.dominant,
    });
    let children: Vec<u32> = node
        .dirs
        .into_iter()
        .map(|child| push(dirs, child))
        .collect();
    dirs[id as usize].children = children;
    id
}

/// How many nodes one answer may hold, and how many children a folder shows.
#[derive(Debug, Clone, Copy)]
pub struct CutLimits {
    pub budget: usize,
    /// For the folder the request is about (the list shows its children, so it gets detail).
    pub first_level: usize,
    /// For every folder below it.
    pub per_folder: usize,
}

impl CutLimits {
    pub const STANDARD: Self = Self {
        budget: SCAN_TREE_BUDGET,
        first_level: SCAN_OPEN_FOLDER_CHILDREN,
        per_folder: SCAN_PER_FOLDER,
    };
}

/// One thing that a folder shows when it is opened.
#[derive(Clone, Copy)]
enum Kid {
    Dir(u32),
    File(usize),
    Other { count: u64, bytes: u64 },
}

/// The tree below folder `start`, cut to the budget. `start_path` is the path to write on the
/// start node (the paths below it are built from the folder names).
pub fn cut(arena: &Arena, start: u32, start_path: &str, limits: &CutLimits) -> DiskNode {
    let mut expanded: HashMap<u32, Vec<Kid>> = HashMap::new();
    // The start folder is one node, collapsed, until it is opened.
    let mut nodes = 1usize;
    // Largest first; on a tie the lower id first, so the result does not change between runs.
    let mut heap: BinaryHeap<(u64, Reverse<u32>)> = BinaryHeap::new();
    heap.push((arena.dirs[start as usize].size, Reverse(start)));

    while let Some((_, Reverse(id))) = heap.pop() {
        let record = &arena.dirs[id as usize];
        let limit = if id == start {
            limits.first_level
        } else {
            limits.per_folder
        };
        let mut candidates = candidates_of(arena, id);
        let total = candidates.len();
        let mut keep = total.min(limit);
        // One entry past the limit is listed, not folded: a node "other (1 item)" costs the same
        // as the entry and says less.
        if total == limit + 1 && record.other_files.0 == 0 {
            keep = total;
        }
        let folded = candidates.split_off(keep);
        let folded_count = folded.len() as u64 + record.other_files.0;
        let folded_bytes: u64 =
            folded.iter().map(|(size, ..)| size).sum::<u64>() + record.other_files.1;
        let cost = keep + usize::from(folded_count > 0);
        // An empty or denied folder has nothing to open. A folder that does not fit stays
        // collapsed and the next biggest one is tried.
        if cost == 0 || nodes + cost > limits.budget {
            continue;
        }
        nodes += cost;
        let mut kids: Vec<Kid> = Vec::with_capacity(cost);
        for (_, _, kid) in candidates {
            if let Kid::Dir(child) = kid {
                heap.push((arena.dirs[child as usize].size, Reverse(child)));
            }
            kids.push(kid);
        }
        if folded_count > 0 {
            kids.push(Kid::Other {
                count: folded_count,
                bytes: folded_bytes,
            });
        }
        expanded.insert(id, kids);
    }
    build(arena, start, start_path, &expanded)
}

/// The sub-folders and kept files of a folder, biggest first (ties by name).
fn candidates_of(arena: &Arena, id: u32) -> Vec<(u64, &str, Kid)> {
    let record = &arena.dirs[id as usize];
    let mut all: Vec<(u64, &str, Kid)> =
        Vec::with_capacity(record.children.len() + record.files.len());
    for &child in &record.children {
        let rec = &arena.dirs[child as usize];
        all.push((rec.size, &rec.name, Kid::Dir(child)));
    }
    for (index, file) in record.files.iter().enumerate() {
        all.push((file.size, &file.name, Kid::File(index)));
    }
    all.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    all
}

fn build(arena: &Arena, id: u32, path: &str, expanded: &HashMap<u32, Vec<Kid>>) -> DiskNode {
    let record = &arena.dirs[id as usize];
    let mut node = DiskNode {
        name: record.name.to_string(),
        path: path.to_owned(),
        size_bytes: record.size,
        kind: NodeKind::Dir,
        category: None,
        children: None,
        truncated: None,
        denied: None,
    };
    if let Some(kids) = expanded.get(&id) {
        let children = kids
            .iter()
            .map(|kid| match *kid {
                Kid::Dir(child) => {
                    let name = &arena.dirs[child as usize].name;
                    build(arena, child, &join(path, name), expanded)
                }
                Kid::File(index) => {
                    let file = &record.files[index];
                    DiskNode {
                        name: file.name.to_string(),
                        path: join(path, &file.name),
                        size_bytes: file.size,
                        kind: NodeKind::File,
                        category: Some(category_of(&file.name)),
                        children: None,
                        truncated: None,
                        denied: None,
                    }
                }
                Kid::Other { count, bytes } => DiskNode {
                    name: other_name(count),
                    path: join(path, "*"),
                    size_bytes: bytes,
                    kind: NodeKind::Other,
                    category: None,
                    children: None,
                    truncated: None,
                    denied: None,
                },
            })
            .collect();
        node.children = Some(children);
    } else if record.denied {
        node.denied = Some(true);
    } else if !record.children.is_empty() || !record.files.is_empty() || record.other_files.0 > 0 {
        // Content that is not in this answer. The category colours its block in the treemap.
        node.truncated = Some(true);
        node.category = Some(category_from_index(record.dominant).unwrap_or(FileCategory::Other));
    }
    node
}

/// "other (1 item)", "other (1,204 items)".
fn other_name(count: u64) -> String {
    if count == 1 {
        "other (1 item)".to_owned()
    } else {
        format!("other ({} items)", with_commas(count))
    }
}

fn with_commas(number: u64) -> String {
    let digits = number.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (position, digit) in digits.chars().enumerate() {
        if position > 0 && (digits.len() - position).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A child's path: the parent, one backslash unless the parent already ends with one, the name.
fn join(parent: &str, name: &str) -> String {
    if parent.ends_with('\\') {
        format!("{parent}{name}")
    } else {
        format!("{parent}\\{name}")
    }
}

/// What a path of a request names.
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup {
    /// A folder of the scan: its index and its path as the arena spells it.
    Dir { id: u32, path: String },
    /// A file, or the `other` node of a folder.
    NotADirectory,
    /// Not in this scan.
    Unknown,
}

/// Finds the folder that `wanted` names. `root_path` is the path of the arena's root (`C:\`).
/// Slashes, a trailing backslash and the case of the letters do not matter; the answer carries
/// the path as the arena spells it, which is what the screen compares byte for byte.
pub fn find(arena: &Arena, root_path: &str, wanted: &str) -> Lookup {
    let wanted = wanted.replace('/', "\\");
    let wanted = wanted.trim_end_matches('\\');
    if wanted.ends_with("\\*") {
        return Lookup::NotADirectory;
    }
    let root = root_path.trim_end_matches('\\');
    if wanted.eq_ignore_ascii_case(root) {
        return Lookup::Dir {
            id: 0,
            path: root_path.to_owned(),
        };
    }
    let below_root = wanted.len() > root.len()
        && wanted.as_bytes()[..root.len()].eq_ignore_ascii_case(root.as_bytes())
        && wanted.as_bytes()[root.len()] == b'\\';
    if !below_root {
        return Lookup::Unknown;
    }
    let segments: Vec<&str> = wanted[root.len() + 1..]
        .split('\\')
        .filter(|segment| !segment.is_empty())
        .collect();
    let mut id = 0u32;
    let mut path = root_path.to_owned();
    for (position, segment) in segments.iter().enumerate() {
        let record = &arena.dirs[id as usize];
        let by = |exact: bool| {
            record.children.iter().copied().find(|&child| {
                let name = &arena.dirs[child as usize].name;
                if exact {
                    **name == **segment
                } else {
                    name.to_lowercase() == segment.to_lowercase()
                }
            })
        };
        match by(true).or_else(|| by(false)) {
            Some(child) => {
                path = join(&path, &arena.dirs[child as usize].name);
                id = child;
            }
            None => {
                let is_last = position + 1 == segments.len();
                let is_file = record
                    .files
                    .iter()
                    .any(|file| file.name.to_lowercase() == segment.to_lowercase());
                return if is_last && is_file {
                    Lookup::NotADirectory
                } else {
                    Lookup::Unknown
                };
            }
        }
    }
    Lookup::Dir { id, path }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> FileNode {
        FileNode {
            name: name.into(),
            size,
        }
    }

    fn dir(name: &str, files: Vec<FileNode>, dirs: Vec<DirNode>) -> DirNode {
        let size =
            files.iter().map(|f| f.size).sum::<u64>() + dirs.iter().map(|d| d.size).sum::<u64>();
        DirNode {
            name: name.into(),
            size,
            files,
            other_files: (0, 0),
            dirs,
            denied: false,
            dominant: 7,
        }
    }

    fn sample() -> Arena {
        // T:\ (a: 600 in 3 files, b\c: 700 in 70 files, big.bin 10 000)
        let a = dir(
            "a",
            vec![file("one", 100), file("two", 200), file("three", 300)],
            vec![],
        );
        let mut c = dir(
            "c",
            (0..64).map(|n| file(&format!("f{n:02}"), 10)).collect(),
            vec![],
        );
        c.other_files = (6, 60);
        c.size += 60;
        let b = dir("b", vec![], vec![c]);
        let empty = dir("empty", vec![], vec![]);
        let mut locked = dir("locked", vec![], vec![]);
        locked.denied = true;
        let root = dir(
            "T:",
            vec![file("big.bin", 10_000), file("notes.TXT", 5)],
            vec![a, b, empty, locked],
        );
        flatten(root)
    }

    /// Checks invariants 1 to 5 of section 3 on a node and everything below it; counts nodes.
    fn check(node: &DiskNode) -> usize {
        let mut count = 1;
        let flags = [
            node.children.is_some(),
            node.truncated == Some(true),
            node.denied == Some(true),
        ];
        assert!(
            flags.iter().filter(|f| **f).count() <= 1,
            "{}: two of three",
            node.path
        );
        if node.kind != NodeKind::Dir {
            assert!(node.children.is_none() && node.truncated.is_none() && node.denied.is_none());
        }
        match node.kind {
            NodeKind::File => assert!(node.category.is_some(), "a file has a category"),
            NodeKind::Other => assert!(node.category.is_none()),
            NodeKind::Dir => {
                if node.denied == Some(true) {
                    assert_eq!(node.size_bytes, 0);
                    assert!(node.category.is_none());
                }
                if node.truncated == Some(true) {
                    assert!(node.category.is_some(), "a truncated folder has a category");
                }
                if node.children.is_some() {
                    assert!(node.category.is_none());
                }
            }
        }
        if let Some(children) = &node.children {
            assert!(!children.is_empty(), "never an empty list");
            let sum: u64 = children.iter().map(|c| c.size_bytes).sum();
            assert_eq!(sum, node.size_bytes, "children add up in {}", node.path);
            assert!(
                children
                    .iter()
                    .filter(|c| c.kind == NodeKind::Other)
                    .count()
                    <= 1
            );
            for child in children {
                count += check(child);
            }
        }
        count
    }

    #[test]
    fn the_whole_sample_fits_and_the_sizes_add_up() {
        let arena = sample();
        let tree = cut(&arena, 0, "T:\\", &CutLimits::STANDARD);
        assert_eq!(tree.size_bytes, 10_000 + 5 + 600 + 700);
        check(&tree);
        let children = tree.children.as_ref().unwrap();
        let b = children.iter().find(|c| c.name == "b").unwrap();
        let c = &b.children.as_ref().unwrap()[0];
        // 64 kept files fall under the per-folder limit of 12: 12 listed, the rest folded.
        let kids = c.children.as_ref().unwrap();
        assert_eq!(kids.len(), 13);
        let other = kids.last().unwrap();
        assert_eq!(other.kind, NodeKind::Other);
        assert_eq!(other.name, "other (58 items)");
        assert_eq!(other.size_bytes, 58 * 10);
        assert_eq!(other.path, "T:\\b\\c\\*");
    }

    #[test]
    fn a_denied_folder_and_an_empty_one_have_their_own_marks() {
        let arena = sample();
        let tree = cut(&arena, 0, "T:\\", &CutLimits::STANDARD);
        let children = tree.children.as_ref().unwrap();
        let locked = children.iter().find(|c| c.name == "locked").unwrap();
        assert_eq!(locked.denied, Some(true));
        assert_eq!(locked.size_bytes, 0);
        let empty = children.iter().find(|c| c.name == "empty").unwrap();
        assert!(empty.children.is_none() && empty.truncated.is_none() && empty.denied.is_none());
    }

    #[test]
    fn the_root_other_node_path_is_the_root_with_a_star() {
        let arena = sample();
        let limits = CutLimits {
            budget: 2000,
            first_level: 2,
            per_folder: 12,
        };
        let tree = cut(&arena, 0, "T:\\", &limits);
        let other = tree.children.as_ref().unwrap().last().unwrap();
        assert_eq!(other.kind, NodeKind::Other);
        assert_eq!(other.path, "T:\\*");
        check(&tree);
    }

    #[test]
    fn one_entry_past_the_limit_is_listed_instead_of_folded() {
        let files: Vec<FileNode> = (0..13).map(|n| file(&format!("f{n:02}"), 10)).collect();
        let arena = flatten(dir("T:", vec![], vec![dir("x", files, vec![])]));
        let tree = cut(&arena, 0, "T:\\", &CutLimits::STANDARD);
        let x = &tree.children.as_ref().unwrap()[0];
        let kids = x.children.as_ref().unwrap();
        assert_eq!(kids.len(), 13);
        assert!(kids.iter().all(|k| k.kind == NodeKind::File));

        // Two past the limit: now they are folded.
        let files: Vec<FileNode> = (0..14).map(|n| file(&format!("f{n:02}"), 10)).collect();
        let arena = flatten(dir("T:", vec![], vec![dir("x", files, vec![])]));
        let tree = cut(&arena, 0, "T:\\", &CutLimits::STANDARD);
        let kids = tree.children.as_ref().unwrap()[0]
            .children
            .as_ref()
            .unwrap();
        assert_eq!(kids.len(), 13);
        assert_eq!(kids.last().unwrap().name, "other (2 items)");
    }

    #[test]
    fn the_other_node_names_count_one_item_and_group_thousands() {
        assert_eq!(other_name(1), "other (1 item)");
        assert_eq!(other_name(6), "other (6 items)");
        assert_eq!(other_name(1204), "other (1,204 items)");
        assert_eq!(other_name(1_234_567), "other (1,234,567 items)");
        assert_eq!(with_commas(999), "999");
        assert_eq!(with_commas(1000), "1,000");
    }

    #[test]
    fn a_folder_that_does_not_fit_stays_collapsed_and_the_next_is_tried() {
        // Root holds a big folder with 10 files and a small one with 2 files. Budget 6:
        // root (1) + big (1) + small (1) = 3 for the root's children, then 3 left.
        // The big folder needs 10 + 0 = 10 nodes (all listed under the per-folder limit 12): no.
        // The small one needs 2: yes.
        let big = dir(
            "big",
            (0..10).map(|n| file(&format!("b{n}"), 100)).collect(),
            vec![],
        );
        let small = dir("small", vec![file("s1", 1), file("s2", 1)], vec![]);
        let arena = flatten(dir("T:", vec![], vec![big, small]));
        let limits = CutLimits {
            budget: 6,
            first_level: 200,
            per_folder: 12,
        };
        let tree = cut(&arena, 0, "T:\\", &limits);
        let kids = tree.children.as_ref().unwrap();
        let big = kids.iter().find(|k| k.name == "big").unwrap();
        let small = kids.iter().find(|k| k.name == "small").unwrap();
        assert_eq!(big.truncated, Some(true));
        assert_eq!(big.category, Some(FileCategory::Other));
        assert!(big.children.is_none());
        assert_eq!(small.children.as_ref().unwrap().len(), 2);
        assert!(check(&tree) <= 6);
    }

    #[test]
    fn the_output_is_the_same_on_every_run_and_ties_are_ordered_by_name() {
        let arena = flatten(dir(
            "T:",
            vec![file("b", 5), file("a", 5), file("c", 5)],
            vec![],
        ));
        let first = cut(&arena, 0, "T:\\", &CutLimits::STANDARD);
        let second = cut(&arena, 0, "T:\\", &CutLimits::STANDARD);
        assert_eq!(first, second);
        let names: Vec<&str> = first
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn a_subtree_is_cut_from_its_own_folder_with_the_first_level_limit() {
        let arena = sample();
        let Lookup::Dir { id, path } = find(&arena, "T:\\", "T:\\b\\c") else {
            panic!("found");
        };
        let limits = CutLimits {
            budget: 2000,
            first_level: 200,
            per_folder: 12,
        };
        let tree = cut(&arena, id, &path, &limits);
        assert_eq!(tree.path, "T:\\b\\c");
        assert_eq!(tree.children.as_ref().unwrap().len(), 64 + 1);
        check(&tree);
    }

    #[test]
    fn a_path_is_found_whatever_its_case_slashes_or_trailing_backslash() {
        let arena = sample();
        let expected = Lookup::Dir {
            id: 2,
            path: "T:\\b".to_owned(),
        };
        // Order of the arena: T:, a, b, c, empty, locked.
        assert_eq!(find(&arena, "T:\\", "T:\\b"), expected);
        assert_eq!(find(&arena, "T:\\", "t:/B/"), expected);
        assert_eq!(
            find(&arena, "T:\\", "T:\\"),
            Lookup::Dir {
                id: 0,
                path: "T:\\".to_owned()
            }
        );
        assert_eq!(find(&arena, "T:\\", "t:"), find(&arena, "T:\\", "T:\\"));
    }

    #[test]
    fn files_other_nodes_and_unknown_paths_are_told_apart() {
        let arena = sample();
        assert_eq!(find(&arena, "T:\\", "T:\\big.bin"), Lookup::NotADirectory);
        assert_eq!(find(&arena, "T:\\", "T:\\BIG.BIN"), Lookup::NotADirectory);
        assert_eq!(find(&arena, "T:\\", "T:\\*"), Lookup::NotADirectory);
        assert_eq!(find(&arena, "T:\\", "T:\\b\\c\\*"), Lookup::NotADirectory);
        assert_eq!(find(&arena, "T:\\", "T:\\nope"), Lookup::Unknown);
        assert_eq!(find(&arena, "T:\\", "T:\\nope\\deeper"), Lookup::Unknown);
        assert_eq!(find(&arena, "T:\\", "U:\\b"), Lookup::Unknown);
        assert_eq!(find(&arena, "T:\\", "T:b"), Lookup::Unknown);
        assert_eq!(find(&arena, "T:\\", "T:\\big.bin\\x"), Lookup::Unknown);
    }

    /// A small deterministic generator, so that the property test needs no new crate.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, below: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % below.max(1)
        }
    }

    fn random_tree(rng: &mut Lcg, folders: usize) -> DirNode {
        // Build a flat list of folders, each with a random parent that came earlier.
        let mut nodes: Vec<DirNode> = (0..folders)
            .map(|n| {
                let file_count = rng.next(30) as usize;
                let files: Vec<FileNode> = (0..file_count)
                    .map(|f| file(&format!("f{f}"), rng.next(5000)))
                    .collect();
                dir(&format!("d{n}"), files, vec![])
            })
            .collect();
        let parents: Vec<usize> = (1..folders).map(|n| rng.next(n as u64) as usize).collect();
        // Attach from the last folder to the first, so that a folder is complete before it moves.
        for n in (1..folders).rev() {
            let mut node = nodes.pop().expect("a folder");
            // A folder that Windows refused has no content, and so no sub-folders.
            if node.dirs.is_empty() && rng.next(10) == 0 {
                node.denied = true;
                node.files.clear();
                node.size = 0;
            }
            let parent = &mut nodes[parents[n - 1]];
            parent.size += node.size;
            parent.dirs.push(node);
        }
        nodes.pop().expect("the root")
    }

    #[test]
    fn random_trees_keep_the_invariants_and_the_budget() {
        let mut rng = Lcg(42);
        for round in 0..500 {
            let folders = 1 + rng.next(if round % 50 == 0 { 5000 } else { 300 }) as usize;
            let arena = flatten(random_tree(&mut rng, folders));
            let budget = 1 + rng.next(300) as usize;
            let limits = CutLimits {
                budget,
                first_level: 1 + rng.next(40) as usize,
                per_folder: 1 + rng.next(12) as usize,
            };
            let tree = cut(&arena, 0, "T:\\", &limits);
            assert!(check(&tree) <= budget, "round {round}: over the budget");
            // And a cut at a random folder.
            let start = rng.next(arena.dirs.len() as u64) as u32;
            let sub = cut(&arena, start, "T:\\x", &limits);
            assert!(
                check(&sub) <= budget,
                "round {round}: subtree over the budget"
            );
            assert_eq!(sub.size_bytes, arena.dirs[start as usize].size);
        }
    }
}
