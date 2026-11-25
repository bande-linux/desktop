#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use bande_status::memory::apps_at_limit;

#[test]
fn files_the_kernel_can_read_again_do_not_count_towards_the_limit_of_the_apps() {
    let slice = tempfile::tempdir().expect("a folder");
    let write = |file: &str, text: &str| fs::write(slice.path().join(file), text).expect(file);
    write("memory.high", "1000\n");
    write("memory.current", "990\n");
    write(
        "memory.stat",
        "anon 400\ninactive_file 350\nactive_file 200\n",
    );
    assert!(!apps_at_limit(slice.path()));
    write(
        "memory.stat",
        "anon 940\ninactive_file 10\nactive_file 10\n",
    );
    assert!(apps_at_limit(slice.path()));
    write("memory.high", "max\n");
    assert!(!apps_at_limit(slice.path()));
}
