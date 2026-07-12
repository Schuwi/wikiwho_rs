// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "cli")]

use std::process::Command;

#[test]
fn zstd_output_is_complete_in_single_and_parallel_modes() {
    let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/exact-regressions/Anontalkpagetext_shortened-manually.xml");
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("wikiwho-cli-zstd-test-{unique}"));
    std::fs::create_dir(&temp_dir).expect("create temporary directory");

    for jobs in ["1", "2"] {
        let plain_path = temp_dir.join(format!("output-{jobs}.jsonl"));
        let zstd_path = temp_dir.join(format!("output-{jobs}.jsonl.zst"));

        for output_path in [&plain_path, &zstd_path] {
            let output = Command::new(env!("CARGO_BIN_EXE_wikiwho-cli"))
                .arg(&input)
                .arg("--output")
                .arg(output_path)
                .arg("--jobs")
                .arg(jobs)
                .arg("--quiet")
                .output()
                .expect("run wikiwho-cli");
            assert!(
                output.status.success(),
                "wikiwho-cli failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let expected = std::fs::read(&plain_path).expect("read uncompressed output");
        let compressed = std::fs::File::open(&zstd_path).expect("open zstd output");
        let decoded = zstd::stream::decode_all(compressed).expect("decode complete zstd frame");
        assert_eq!(decoded, expected, "zstd output differs with --jobs {jobs}");
    }

    std::fs::remove_dir_all(temp_dir).expect("remove temporary directory");
}
