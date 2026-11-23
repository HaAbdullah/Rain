//! Finding the projects: every git repository directly inside a project folder. Section 02 6.2.
//!
//! Only direct children count. The root itself is not a project, a folder without `.git` is not
//! one, and a repository two levels down is not found (the user adds that folder as a root). This
//! runs on the blocking pool; it costs one `read_dir` per root and one `metadata` per entry.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use super::git::{head_signature, read_branch};
use super::ids::{norm_path, path_key};
use super::store::Found;

/// What one pass over the roots found.
#[derive(Debug, Default)]
pub struct Discovery {
    pub found: Vec<Found>,
    /// Roots that could not be read (missing, no access), as the user wrote them.
    pub unreadable: Vec<String>,
}

pub fn discover(roots: &[PathBuf]) -> Discovery {
    let mut out = Discovery::default();
    let mut seen_roots = HashSet::new();
    let mut seen_projects = HashSet::new();
    for root in roots {
        let root_text = root.to_string_lossy();
        let root_key = path_key(&root_text);
        if !seen_roots.insert(root_key.clone()) {
            continue;
        }
        let Ok(entries) = fs::read_dir(root) else {
            out.unreadable.push(norm_path(&root_text));
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Hidden and system folders are never projects.
            if name.starts_with('.') || name.starts_with('$') {
                continue;
            }
            let path = entry.path();
            // `metadata` follows junctions and links; a file is not a project.
            if !fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
                continue;
            }
            // `.git` is a folder in a repository and a file in a worktree or a submodule.
            if fs::metadata(path.join(".git")).is_err() {
                continue;
            }
            let text = norm_path(&path.to_string_lossy());
            let key = path_key(&text);
            if !seen_projects.insert(key.clone()) {
                continue;
            }
            out.found.push(Found {
                branch: read_branch(&path),
                sig: head_signature(&path),
                path: text,
                key,
                name,
                root_key: root_key.clone(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(parent: &std::path::Path, name: &str, head: &str) {
        let git = parent.join(name).join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), head).unwrap();
    }

    #[test]
    fn repositories_directly_inside_a_root_are_found() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();

        repo(&a, "alpha", "ref: refs/heads/main\n");
        repo(&a, "beta", &"ab".repeat(20));
        // A worktree: `.git` is a file that points to a folder.
        let store = temp.path().join("store");
        fs::create_dir_all(&store).unwrap();
        fs::write(store.join("HEAD"), "ref: refs/heads/topic\n").unwrap();
        fs::create_dir_all(a.join("gamma")).unwrap();
        fs::write(
            a.join("gamma").join(".git"),
            format!("gitdir: {}\n", store.display()),
        )
        .unwrap();
        // Not projects: no `.git`, hidden, a loose file, a repository one level deeper.
        fs::create_dir_all(a.join("delta")).unwrap();
        repo(&a, ".hidden", "ref: refs/heads/x\n");
        repo(&a, "$sys", "ref: refs/heads/x\n");
        fs::write(a.join("loose.txt"), "x").unwrap();
        repo(&a.join("delta"), "inner", "ref: refs/heads/x\n");
        repo(&b, "alpha", "ref: refs/heads/dev\n");

        let missing = temp.path().join("missing");
        let found = discover(&[a.clone(), b.clone(), missing.clone(), a.clone()]);

        let mut summary: Vec<(String, String)> = found
            .found
            .iter()
            .map(|f| (f.name.clone(), f.branch.clone()))
            .collect();
        summary.sort();
        assert_eq!(
            summary,
            vec![
                ("alpha".to_owned(), "dev".to_owned()),
                ("alpha".to_owned(), "main".to_owned()),
                ("beta".to_owned(), "abababa".to_owned()),
                ("gamma".to_owned(), "topic".to_owned()),
            ]
        );
        assert_eq!(
            found.unreadable,
            vec![norm_path(&missing.to_string_lossy())]
        );
        // The key is the lower-case path and the root is remembered.
        let first = &found.found[0];
        assert_eq!(first.key, first.path.to_lowercase());
        assert!(first.key.starts_with(&first.root_key));
    }
}
