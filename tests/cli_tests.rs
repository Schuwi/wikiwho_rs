// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "cli")]

use std::process::Command;

#[test]
fn invalid_compression_level_does_not_truncate_existing_output() {
    let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/exact-regressions/Anontalkpagetext_shortened-manually.xml");
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("wikiwho-cli-test-{unique}"));
    std::fs::create_dir(&temp_dir).expect("create temporary directory");
    let original_contents = b"existing output must be preserved";

    for (extension, level) in [("bz2", "0"), ("gz", "10"), ("zst", "23")] {
        let output_path = temp_dir.join(format!("output.{extension}"));
        std::fs::write(&output_path, original_contents).expect("write existing output");

        let output = Command::new(env!("CARGO_BIN_EXE_wikiwho-cli"))
            .arg(&input)
            .arg("--output")
            .arg(&output_path)
            .arg("--compression-level")
            .arg(level)
            .output()
            .expect("run wikiwho-cli");

        assert!(
            !output.status.success(),
            "{extension} level should be rejected"
        );
        assert_eq!(
            std::fs::read(&output_path).expect("read existing output"),
            original_contents,
            "invalid {extension} level truncated the output"
        );
    }

    std::fs::remove_dir_all(temp_dir).expect("remove temporary directory");
}
