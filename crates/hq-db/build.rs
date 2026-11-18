//! Lists the SQL files in `sql/` for `src/migrations.rs`.
//!
//! The list used to be written by hand, one entry per file, so every pull request that added a
//! migration edited the same place in the same 700 line array and two of them always collided.
//! Now a migration is just a new file: this script writes `OUT_DIR/migrations_list.rs`, a
//! `&[(&str, &str)]` of (file name without `.sql`, the file's text) in file name order, and
//! `migrations.rs` includes it. It only lists files and `include_str!`s them: nothing is
//! generated from their content, so there is nothing to keep in step.
//!
//! Two pull requests that each add `140_*.sql` no longer conflict in git; the test
//! `migration_numbers_are_unique_and_in_order` in `src/migrations.rs` fails loudly instead.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let out_dir = env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    let sql = Path::new(&manifest).join("sql");

    // A new or removed file in the folder reruns this script; a changed file reruns the compile
    // through its own include_str!.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=sql");

    let mut names: Vec<String> = fs::read_dir(&sql)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", sql.display()))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".sql"))
        .collect();
    // Plain byte order: `012_a` before `012_b`, `099_` before `100_`. The order is the order the
    // migrations run in.
    names.sort();

    let mut list = String::from("&[\n");
    for name in &names {
        let stem = &name[..name.len() - ".sql".len()];
        list.push_str(&format!(
            "    ({stem:?}, include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/sql/{name}\"))),\n"
        ));
    }
    list.push_str("]\n");

    fs::write(Path::new(&out_dir).join("migrations_list.rs"), list)
        .expect("write the migrations list");
}
