//! Dev tool: run `parse_description_yml` over a directory of real
//! `description.yml` files and report what it makes of each.
//!
//! Usage: `cargo run --example parse_descriptions -- <dir>`

use std::fs;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: parse_descriptions <dir>");
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "yml"))
        .collect();
    entries.sort();

    let (mut ok, mut err) = (0, 0);
    for path in entries {
        let content = fs::read_to_string(&path).expect("read file");
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        match quack_rs::validate::description_yml::parse_description_yml(&content) {
            Ok(d) => {
                ok += 1;
                let mismatch = if d.name == stem {
                    ""
                } else {
                    "  <-- NAME MISMATCH"
                };
                println!(
                    "OK   {stem:<16} name={:<16} version={:<10} lang={:<6} build={:<8} lic={}{mismatch}",
                    d.name, d.version, d.language, d.build, d.license
                );
            }
            Err(e) => {
                err += 1;
                println!("FAIL {stem:<16} {e}");
            }
        }
    }
    println!("\n{ok} parsed, {err} rejected");
}
