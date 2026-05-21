//! ULID-based knowledge IDs. Sortable, URL-safe, 26 chars.

use ulid::Ulid;

pub fn new_id() -> String {
    Ulid::new().to_string()
}
