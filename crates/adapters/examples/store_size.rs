//! cargo run --release -p diffz-adapters --example store_size -- SNAPSHOT.json COUNT
//! SNAPSHOT.json is `diffz --inspect` output; COUNT distinct revisions of it are stored.
use diffz_adapters::store::Store;
use diffz_core::domain::Snapshot;
use std::{fs, path::Path, time::Instant};

fn size(dir: &Path) -> u64 {
    ["review.sqlite3", "review.sqlite3-wal"]
        .iter()
        .filter_map(|f| fs::metadata(dir.join(f)).ok())
        .map(|m| m.len())
        .sum()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let base: Snapshot = serde_json::from_slice(&fs::read(&args[0]).unwrap()).unwrap();
    let count: usize = args[1].parse().unwrap();
    let dir = std::env::temp_dir().join(format!("diffz-store-size-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);

    let started = Instant::now();
    let mut last = None;
    {
        let store = Store::open(&dir).unwrap();
        for i in 0..count {
            let s = Snapshot::with_origin(
                base.title.clone(),
                base.patch.clone(),
                base.remote.clone(),
                base.comments.clone(),
                format!("{}#{i}", base.origin),
            );
            store.put_snapshot(&s).unwrap();
            store.show_recent(&s.id).unwrap();
            last = Some(s.id);
        }
    }
    let written = started.elapsed();
    let before = size(&dir);

    let started = Instant::now();
    let store = Store::open(&dir).unwrap();
    let reopen = started.elapsed();
    let last = last.unwrap();
    let started = Instant::now();
    for _ in 0..10 {
        store.snapshot(&last).unwrap();
    }
    let load = started.elapsed() / 10;
    drop(store);
    let after = size(&dir);

    println!(
        "{count} revisions: store {:.1} MB after writing ({written:.2?}), {:.1} MB after reopening ({reopen:.2?}); newest loads in {load:.2?}",
        before as f64 / 1e6,
        after as f64 / 1e6,
    );
    let _ = fs::remove_dir_all(&dir);
}
