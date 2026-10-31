//! What `integration` exposes to the rest of the workspace. Nothing in the domain graph depends
//! on integration; the one file here is a slot another domain fills at start-up
//! (`for_partner.rs`).

pub mod for_partner;
