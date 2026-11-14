//! Linux: Rust panics already print where they happened, and there is no crash dialog to
//! suppress, so headless runs need nothing installed. `UTSURI_CRASH_TEST=1` still crashes on
//! purpose, so the test runners' crash handling can be checked.

pub fn install(_quiet: bool) {}

pub fn self_test() {
    if std::env::var_os("UTSURI_CRASH_TEST").is_some() {
        std::process::abort();
    }
}
