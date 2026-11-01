//! Browser-owned native messaging process. Stdout is reserved for framed JSON.

fn main() {
    if lockbox::browser::run().is_err() {
        // Never print requests, passwords, file paths or vault contents.
        std::process::exit(1);
    }
}
