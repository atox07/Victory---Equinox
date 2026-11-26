//! Synthetic native process: checks launcher argv and writes numeric test data.
//! It never executes, maps or reads a real title.
use std::{env, fs, path::Path};

fn main() {
    let a: Vec<String> = env::args().skip(1).collect();
    assert_eq!(a.len(), 5);
    assert_eq!(a[0], "run");
    assert_eq!(a[2], "--execute");
    assert_eq!(a[3], "--console");
    assert_eq!(a[4], "series-s");
    let title = Path::new(&a[1]);
    assert_eq!(title.file_name().unwrap(), "synthetic title.exe");
    assert_eq!(fs::read_to_string(title).unwrap(), "synthetic placeholder");
    let game_dir = env::var("VICTORY_GAME_DIR").unwrap();
    assert_eq!(title.parent().unwrap(), Path::new(&game_dir));
    assert_eq!(env::var("VICTORY_SMART_COPY_BACK").unwrap(), "0");
    assert!(env::var_os("VICTORY_SYNTHETIC_PRIVATE").is_none());
    fs::write(env::var_os("VICTORY_PRESENT_CSV").unwrap(),
        "elapsed_us,frametime_us\n10000,10000\n20000,10000\n").unwrap();
    println!("EQUINOX_SYNTHETIC_ARGV_OK");
}
