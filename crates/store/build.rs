//! `sqlx::migrate!` embeds `migrations/` at compile time; rebuild when a migration is added or
//! changed, not only when this crate's sources change.

fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
