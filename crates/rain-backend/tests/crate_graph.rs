//! The dependency rules of foundation section 1.2, checked against `cargo metadata`.
//!
//! 1. `rain-types` depends on no other crate of the workspace.
//! 2. Nothing depends on `rain-backend` except the backend itself (its tests).
//! 3. `rain-hook` depends on no crate at all.
//! 4. Nobody depends on `rain` (the shell).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// Every package of the workspace and the names of the crates it depends on (normal, dev and
/// build dependencies together).
fn dependencies_by_package(metadata: &Value) -> BTreeMap<String, Vec<String>> {
    metadata["packages"]
        .as_array()
        .expect("`packages` is a list")
        .iter()
        .map(|package| {
            let name = package["name"].as_str().expect("a package name").to_owned();
            let dependencies = package["dependencies"]
                .as_array()
                .expect("`dependencies` is a list")
                .iter()
                .map(|dependency| dependency["name"].as_str().expect("a name").to_owned())
                .collect();
            (name, dependencies)
        })
        .collect()
}

/// The rules. Returns one sentence for every violation.
fn violations(packages: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let workspace: Vec<&String> = packages.keys().collect();
    let mut found = Vec::new();

    for (name, dependencies) in packages {
        let uses = |other: &str| dependencies.iter().any(|d| d == other);

        if name == "rain-types" {
            for dependency in dependencies.iter().filter(|d| workspace.contains(d)) {
                found.push(format!("rain-types depends on {dependency}"));
            }
        }
        if name == "rain-hook" && !dependencies.is_empty() {
            found.push(format!("rain-hook depends on {}", dependencies.join(", ")));
        }
        if name != "rain-backend" && uses("rain-backend") {
            found.push(format!("{name} depends on rain-backend"));
        }
        if uses("rain") {
            found.push(format!("{name} depends on rain (the shell)"));
        }
    }
    found
}

fn graph(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    entries
        .iter()
        .map(|(name, deps)| {
            (
                (*name).to_owned(),
                deps.iter().map(|d| (*d).to_owned()).collect(),
            )
        })
        .collect()
}

#[test]
fn the_workspace_follows_the_dependency_rules() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    // clippy.toml forbids `Command::new` in the backend; this test is not the backend. It asks
    // the same cargo that runs the test, and reads only the workspace's own packages.
    #[allow(clippy::disallowed_methods)]
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(&manifest)
        .output()
        .expect("cargo metadata could be started");
    assert!(output.status.success(), "cargo metadata failed");

    let metadata: Value = serde_json::from_slice(&output.stdout).expect("metadata is JSON");
    let packages = dependencies_by_package(&metadata);
    assert!(
        packages.contains_key("rain-backend"),
        "the scan found the workspace"
    );

    let found = violations(&packages);
    assert!(found.is_empty(), "dependency rules broken: {found:?}");
}

// A rule that can never fail proves nothing, so each rule is also tried on a graph that breaks it.

#[test]
fn a_clean_graph_has_no_violation() {
    let clean = graph(&[
        ("rain-types", &["serde"]),
        ("rain-pipe", &["rain-types", "tokio"]),
        ("rain-backend", &["rain-types", "rain-pipe"]),
        ("rain-hook", &[]),
        ("rain", &["rain-types", "rain-pipe"]),
    ]);
    assert!(violations(&clean).is_empty());
}

#[test]
fn rain_types_may_not_use_another_crate_of_the_workspace() {
    let broken = graph(&[("rain-types", &["serde", "rain-pipe"]), ("rain-pipe", &[])]);
    assert_eq!(violations(&broken), ["rain-types depends on rain-pipe"]);
}

#[test]
fn only_the_backend_may_use_the_backend() {
    let broken = graph(&[("rain-backend", &[]), ("rain", &["rain-backend"])]);
    assert_eq!(violations(&broken), ["rain depends on rain-backend"]);
}

#[test]
fn the_hook_has_no_dependencies() {
    let broken = graph(&[("rain-hook", &["serde_json"])]);
    assert_eq!(violations(&broken), ["rain-hook depends on serde_json"]);
}

#[test]
fn nobody_may_use_the_shell() {
    let broken = graph(&[("rain", &[]), ("rain-pipe", &["rain"])]);
    assert_eq!(
        violations(&broken),
        ["rain-pipe depends on rain (the shell)"]
    );
}
