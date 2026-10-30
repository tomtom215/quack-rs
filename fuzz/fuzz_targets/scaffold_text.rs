// SPDX-License-Identifier: MIT
//! Fuzzes the scaffold's escaping: for any free text `generate_scaffold`
//! accepts, `description.yml` must parse back to exactly that text, and every
//! description line must stay inside the `//!` header of `src/lib.rs`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quack_rs::scaffold::{generate_scaffold, ScaffoldConfig};
use quack_rs::validate::description_yml::parse_description_yml;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let (description, maintainer) = text.split_once('\u{1}').unwrap_or((text, "Jane Doe"));
    let config = ScaffoldConfig {
        name: "my_ext".to_string(),
        description: description.to_string(),
        maintainer: maintainer.to_string(),
        github_repo: "janedoe/duckdb-my-ext".to_string(),
        ..ScaffoldConfig::default()
    };
    let Ok(files) = generate_scaffold(&config) else {
        return;
    };
    let file = |p: &str| files.iter().find(|f| f.path == p).expect(p).content.clone();
    let yml = file("description.yml");
    let parsed =
        parse_description_yml(&yml).unwrap_or_else(|e| panic!("{description:?}: {e}\n{yml}"));
    assert_eq!(parsed.description, description, "{yml}");
    assert_eq!(parsed.maintainers, vec![maintainer.to_string()], "{yml}");
    let lib = file("src/lib.rs");
    let header: Vec<&str> = lib
        .split('\n')
        .take_while(|l| !l.starts_with("use "))
        .filter(|l| !l.is_empty())
        .collect();
    assert!(
        header.iter().all(|l| l.starts_with("//!")),
        "{description:?}\n{lib}"
    );
    assert!(!lib.contains('\r'));
});
