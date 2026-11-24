use std::error::Error;
use std::hint::black_box;
use std::thread;

fn anonymous_bytes() -> Result<usize, Box<dyn Error>> {
    let rollup = std::fs::read_to_string("/proc/self/smaps_rollup")?;
    let kib = rollup
        .lines()
        .find_map(|line| line.strip_prefix("Anonymous:")?.split_whitespace().next())
        .ok_or("no Anonymous line")?;
    Ok(kib.parse::<usize>()? * 1024)
}

#[test]
fn a_freed_picture_leaves_no_memory_behind() -> Result<(), Box<dyn Error>> {
    const PICTURE: usize = 20 << 20;
    bande_log::init("bande-log-test");
    let before = anonymous_bytes()?;
    for _ in 0..2 {
        thread::scope(|scope| {
            scope.spawn(|| drop(black_box(vec![1u8; PICTURE])));
        });
    }
    let kept = anonymous_bytes()?.saturating_sub(before);
    assert!(kept < PICTURE / 4, "{kept} bytes of freed memory are kept");
    Ok(())
}
