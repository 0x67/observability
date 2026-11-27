//! The README `## Compatibility` table checked against `Cargo.lock` and the manifests.
//!
//! The newest row describes the newest or the upcoming release, so it must match what the repo
//! builds today, and it must not be older than this crate's version.

use std::collections::BTreeSet;

const README: &str = include_str!("../../../README.md");
const LOCK: &str = include_str!("../../../Cargo.lock");
const WORKSPACE_MANIFEST: &str = include_str!("../../../Cargo.toml");
const CORE_MANIFEST: &str = include_str!("../../observability-core/Cargo.toml");
const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

const COLUMNS: [&str; 10] = [
    "observability tag",
    "observability-core",
    "opentelemetry family",
    "tracing-opentelemetry",
    "metrics-exporter-otel",
    "metrics",
    "metrics-exporter-prometheus",
    "tracing / tracing-subscriber",
    "tonic",
    "MSRV",
];

/// Columns that show locked major.minor versions, with the crates behind each ` / `-separated
/// part of the cell. Every crate in one part must lock the same major.minor.
const LOCKED_COLUMNS: [(&str, &[&[&str]]); 6] = [
    (
        "opentelemetry family",
        &[&[
            "opentelemetry",
            "opentelemetry_sdk",
            "opentelemetry-otlp",
            "opentelemetry-appender-tracing",
            "opentelemetry-http",
            "opentelemetry-proto",
        ]],
    ),
    ("tracing-opentelemetry", &[&["tracing-opentelemetry"]]),
    ("metrics", &[&["metrics"]]),
    (
        "metrics-exporter-prometheus",
        &[&["metrics-exporter-prometheus"]],
    ),
    (
        "tracing / tracing-subscriber",
        &[&["tracing"], &["tracing-subscriber"]],
    ),
    ("tonic", &[&["tonic"]]),
];

/// Locked until the in-crate OTLP metrics recorder replaced it; the cell then reads `in-crate`.
const OTEL_BRIDGE: &str = "metrics-exporter-otel";

/// Every version `Cargo.lock` records for `name`. Lock format 4 puts `version` on the line
/// right after `name` in each `[[package]]` block.
fn locked_versions<'a>(lock: &'a str, name: &str) -> Vec<&'a str> {
    let name_line = format!("name = \"{name}\"");
    lock.lines()
        .zip(lock.lines().skip(1))
        .filter(|(line, _)| *line == name_line)
        .filter_map(|(_, next)| next.strip_prefix("version = \"")?.strip_suffix('"'))
        .collect()
}

fn major_minor(version: &str) -> &str {
    let end = version
        .match_indices('.')
        .nth(1)
        .map_or(version.len(), |(dot, _)| dot);
    &version[..end]
}

fn numeric_version(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| {
            part.parse()
                .unwrap_or_else(|_| panic!("`{version}` is not a plain x.y.z version"))
        })
        .collect()
}

/// The cell a locked column should hold. A part whose crates lock different minors lists
/// all of them, so the failure shows the split.
fn locked_cell(parts: &[&[&str]]) -> String {
    parts
        .iter()
        .map(|crates| {
            let minors: BTreeSet<&str> = crates
                .iter()
                .flat_map(|name| locked_versions(LOCK, name))
                .map(major_minor)
                .collect();
            minors.into_iter().collect::<Vec<_>>().join(", ")
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

/// The first `key = "value"` line, which for `version` and `rust-version` is the package or
/// workspace one: dependency versions sit in inline tables.
fn manifest_value<'a>(manifest: &'a str, key: &str) -> &'a str {
    manifest
        .lines()
        .find_map(|line| {
            line.strip_prefix(key)?
                .strip_prefix(" = \"")?
                .strip_suffix('"')
        })
        .unwrap_or_else(|| panic!("no `{key}` line in the manifest"))
}

/// The table's lines as trimmed cells: header, separator, then one row per tag.
fn compat_rows(readme: &str) -> Vec<Vec<&str>> {
    readme
        .lines()
        .skip_while(|line| *line != "## Compatibility")
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'))
        .map(|line| line.trim_matches('|').split('|').map(str::trim).collect())
        .collect()
}

fn newest_row<'r, 'a>(rows: &'r [Vec<&'a str>]) -> &'r [&'a str] {
    assert!(
        rows.len() > 2,
        "README needs a `## Compatibility` table with a header and at least one row"
    );
    assert_eq!(
        rows[0], COLUMNS,
        "README `## Compatibility` header must list these columns in this order"
    );
    &rows[rows.len() - 1]
}

fn tag_version<'a>(row: &[&'a str]) -> &'a str {
    row[0].strip_prefix("observability-v").unwrap_or_else(|| {
        panic!(
            "column `observability tag`: README has `{}`, expected `observability-v<x.y.z>`",
            row[0]
        )
    })
}

// @lat: [[tests#Compatibility table#Newest row matches the lockfile]]
#[test]
fn newest_readme_row_matches_lockfile() {
    let rows = compat_rows(README);
    let row = newest_row(&rows);
    assert_eq!(
        row.len(),
        COLUMNS.len(),
        "row `{}` has the wrong cell count",
        row[0]
    );

    let otel_bridge = locked_cell(&[&[OTEL_BRIDGE]]);
    let otel_bridge = if otel_bridge.is_empty() {
        "in-crate".to_owned()
    } else {
        otel_bridge
    };
    let mut expected = vec![
        (OTEL_BRIDGE, otel_bridge, "Cargo.lock"),
        (
            "MSRV",
            manifest_value(WORKSPACE_MANIFEST, "rust-version").to_owned(),
            "Cargo.toml rust-version",
        ),
    ];
    expected.extend(
        LOCKED_COLUMNS
            .iter()
            .map(|(column, parts)| (*column, locked_cell(parts), "Cargo.lock")),
    );
    // Before a release the newest row names the upcoming tag, but the core version moves only
    // in the release pull request, so the core cell is checked from that pull request on.
    if tag_version(row) == CRATE_VERSION {
        expected.push((
            "observability-core",
            manifest_value(CORE_MANIFEST, "version").to_owned(),
            "the observability-core Cargo.toml",
        ));
    }

    let mismatches: Vec<String> = expected
        .iter()
        .filter_map(|(column, want, source)| {
            let index = COLUMNS
                .iter()
                .position(|name| name == column)
                .expect("every checked column is in COLUMNS");
            (row[index] != want.as_str()).then(|| {
                format!(
                    "column `{column}`: README has `{}`, {source} has `{want}`",
                    row[index]
                )
            })
        })
        .collect();
    assert!(
        mismatches.is_empty(),
        "README row `{}` disagrees with the build:\n{}",
        row[0],
        mismatches.join("\n")
    );
}

// @lat: [[tests#Compatibility table#Newest row covers the crate version]]
#[test]
fn newest_readme_row_covers_crate_version() {
    let rows = compat_rows(README);
    let row = newest_row(&rows);

    assert!(
        numeric_version(tag_version(row)) >= numeric_version(CRATE_VERSION),
        "the newest README compatibility row is `{}`; add a row for `observability-v{CRATE_VERSION}`",
        row[0]
    );
}

// @lat: [[tests#Compatibility table#Tracked crates have one locked version]]
#[test]
fn tracked_crates_have_one_locked_version() {
    let failures: Vec<String> = LOCKED_COLUMNS
        .iter()
        .flat_map(|(_, parts)| parts.iter().copied().flatten())
        .map(|name| (name, locked_versions(LOCK, name)))
        .filter(|(_, versions)| versions.len() != 1)
        .map(|(name, versions)| format!("`{name}` is locked at {versions:?}"))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));

    let bridge = locked_versions(LOCK, OTEL_BRIDGE);
    assert!(bridge.len() <= 1, "`{OTEL_BRIDGE}` is locked at {bridge:?}");
}

// @lat: [[tests#Compatibility table#Lock parser reports every version]]
#[test]
fn locked_versions_reports_every_version() {
    let lock = r#"version = 4

[[package]]
name = "tonic"
version = "0.13.1"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "tonic-build"
version = "0.14.6"

[[package]]
name = "tonic"
version = "0.14.6"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

    assert_eq!(locked_versions(lock, "tonic"), ["0.13.1", "0.14.6"]);
}
