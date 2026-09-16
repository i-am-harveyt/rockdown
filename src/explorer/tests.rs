use super::*;

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        Self(unique_directory(&std::env::temp_dir(), "rockdown-explorer-test").unwrap())
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn rename_line(explorer: &mut Explorer, old: &str, new: &str) {
    explorer
        .buffer
        .lines
        .iter_mut()
        .find(|line| line.text == old)
        .unwrap()
        .text = new.to_owned();
}

#[test]
fn creates_renames_and_recovers_deleted_entries() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join("old"), "keep contents").unwrap();
    fs::create_dir(sandbox.0.join("removed")).unwrap();
    fs::write(sandbox.0.join("removed/child"), "recover me").unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    rename_line(&mut explorer, "old", "renamed");
    explorer.buffer.lines.retain(|line| line.text != "removed/");
    explorer.buffer.row = 0;
    explorer.buffer.key("A");
    explorer.buffer.insert_text("\nnew-file\nnew-directory/");
    let report = explorer.commit().unwrap();
    assert_eq!(
        fs::read_to_string(sandbox.0.join("renamed")).unwrap(),
        "keep contents"
    );
    assert!(sandbox.0.join("new-file").is_file());
    assert!(sandbox.0.join("new-directory").is_dir());
    assert!(!sandbox.0.join("old").exists());
    assert!(!sandbox.0.join("removed").exists());
    assert_eq!(
        report.renamed,
        vec![(
            explorer.directory.join("old"),
            explorer.directory.join("renamed")
        )]
    );
    assert_eq!(report.deleted, vec![explorer.directory.join("removed")]);
    assert_eq!(
        report.created,
        vec![
            explorer.directory.join("new-file"),
            explorer.directory.join("new-directory")
        ]
    );
    let transaction = fs::read_dir(sandbox.0.join(TRASH))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        fs::read_to_string(transaction.join("removed/child")).unwrap(),
        "recover me"
    );
    assert!(!explorer.buffer.dirty());
}

#[test]
fn rejects_traversal_duplicates_and_type_changes_before_mutation() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join("original"), "untouched").unwrap();
    fs::write(sandbox.0.join("other"), "other contents").unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    for invalid in [
        "..",
        ".",
        "../escape",
        "/absolute",
        "nested/name",
        TRASH,
        "other",
        "original/",
    ] {
        let line = explorer
            .buffer
            .lines
            .iter_mut()
            .find(|line| line.text != "other")
            .unwrap();
        line.text = invalid.to_owned();
        assert!(explorer.commit().is_err(), "accepted {invalid:?}");
        assert_eq!(
            fs::read_to_string(sandbox.0.join("original")).unwrap(),
            "untouched"
        );
        assert_eq!(
            fs::read_to_string(sandbox.0.join("other")).unwrap(),
            "other contents"
        );
        explorer.buffer.lines[0].text = "original".to_owned();
    }
}

#[test]
fn rename_swap_preserves_both_files() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join("a"), "A").unwrap();
    fs::write(sandbox.0.join("b"), "B").unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    explorer.buffer.lines[0].text = "b".into();
    explorer.buffer.lines[1].text = "a".into();
    let report = explorer.commit().unwrap();
    assert_eq!(fs::read_to_string(sandbox.0.join("a")).unwrap(), "B");
    assert_eq!(fs::read_to_string(sandbox.0.join("b")).unwrap(), "A");
    assert_eq!(report.renamed.len(), 2);
}

#[test]
fn external_collision_and_content_change_abort_without_mutation() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join("source"), "original").unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    rename_line(&mut explorer, "source", "destination");
    fs::write(sandbox.0.join("destination"), "external").unwrap();
    assert!(explorer.commit().is_err());
    assert_eq!(
        fs::read_to_string(sandbox.0.join("destination")).unwrap(),
        "external"
    );
    assert_eq!(
        fs::read_to_string(sandbox.0.join("source")).unwrap(),
        "original"
    );
    fs::remove_file(sandbox.0.join("destination")).unwrap();
    fs::write(sandbox.0.join("source"), "externally modified contents").unwrap();
    assert!(explorer.commit().is_err());
    assert!(!sandbox.0.join("destination").exists());
}

#[test]
fn lists_hidden_files_and_refuses_dirty_navigation() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join(".hidden"), "").unwrap();
    fs::create_dir(sandbox.0.join("z-dir")).unwrap();
    fs::create_dir(sandbox.0.join(TRASH)).unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    assert_eq!(explorer.buffer.text(), "z-dir/\n.hidden");
    explorer.buffer.row = 1;
    explorer.buffer.col = 0;
    explorer.buffer.insert_text("changed-");
    assert!(explorer.enter().is_err());
    assert!(explorer.parent().is_err());
    assert!(explorer.reload().is_err());
}

#[cfg(unix)]
#[test]
fn deleting_directory_symlink_never_touches_its_target() {
    let sandbox = Sandbox::new();
    let target = Sandbox::new();
    fs::write(target.0.join("precious"), "safe").unwrap();
    std::os::unix::fs::symlink(&target.0, sandbox.0.join("link")).unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    explorer.buffer.lines[0].text.clear();
    explorer.commit().unwrap();
    assert_eq!(
        fs::read_to_string(target.0.join("precious")).unwrap(),
        "safe"
    );
    let transaction = fs::read_dir(sandbox.0.join(TRASH))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(
        fs::symlink_metadata(transaction.join("link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn rejects_non_utf8_names() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let sandbox = Sandbox::new();
    let invalid = sandbox.0.join(OsString::from_vec(vec![0xff]));
    fs::write(&invalid, "").unwrap();
    assert!(Explorer::open(&sandbox.0).is_err());
}

#[test]
fn rejects_multiline_names() {
    assert!(parse_name("two\nlines").is_err());
    // Win32 rejects this name before the explorer can enumerate it.
    #[cfg(not(windows))]
    {
        let sandbox = Sandbox::new();
        fs::write(sandbox.0.join("two\nlines"), "").unwrap();
        assert!(Explorer::open(&sandbox.0).is_err());
    }
}

#[test]
fn no_replace_moves_preserve_existing_files_and_directories() {
    let sandbox = Sandbox::new();
    for directory in [false, true] {
        let source = sandbox
            .0
            .join(if directory { "source-dir" } else { "source" });
        let destination = sandbox
            .0
            .join(if directory { "target-dir" } else { "target" });
        if directory {
            fs::create_dir(&source).unwrap();
            fs::create_dir(&destination).unwrap();
            fs::write(source.join("contents"), "source").unwrap();
        } else {
            fs::write(&source, "source").unwrap();
            fs::write(&destination, "external").unwrap();
        }
        assert!(move_without_overwrite(&source, &destination).is_err());
        if directory {
            assert_eq!(
                fs::read_to_string(source.join("contents")).unwrap(),
                "source"
            );
            assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
            fs::remove_dir(&destination).unwrap();
        } else {
            assert_eq!(fs::read_to_string(&source).unwrap(), "source");
            assert_eq!(fs::read_to_string(&destination).unwrap(), "external");
            fs::remove_file(&destination).unwrap();
        }
        move_without_overwrite(&source, &destination).unwrap();
        assert!(!source.exists());
        let contents = if directory {
            destination.join("contents")
        } else {
            destination
        };
        assert_eq!(fs::read_to_string(contents).unwrap(), "source");
    }
}

#[test]
fn case_only_renames_use_staging_for_files_and_directories() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join("file"), "contents").unwrap();
    fs::create_dir(sandbox.0.join("folder")).unwrap();
    fs::write(sandbox.0.join("folder/child"), "child").unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    rename_line(&mut explorer, "file", "FILE");
    rename_line(&mut explorer, "folder/", "FOLDER/");
    let report = explorer.commit().unwrap();
    assert_eq!(report.renamed.len(), 2);
    assert_eq!(explorer.buffer.text(), "FOLDER/\nFILE");
    assert_eq!(
        fs::read_to_string(sandbox.0.join("FILE")).unwrap(),
        "contents"
    );
    assert_eq!(
        fs::read_to_string(sandbox.0.join("FOLDER/child")).unwrap(),
        "child"
    );
    assert_eq!(fs::read_dir(&sandbox.0).unwrap().count(), 2);
}

#[test]
fn rollback_preserves_external_obstacles_and_staged_recovery_data() {
    let sandbox = Sandbox::new();
    let staged = sandbox.0.join("staged");
    let source = sandbox.0.join("source");
    fs::write(&staged, "original").unwrap();
    fs::write(&source, "external").unwrap();
    let mut operations = [Operation {
        source: Some(source.clone()),
        staged: staged.clone(),
        destination: sandbox.0.join("destination"),
        directory: false,
        location: Location::Staged,
    }];
    assert_eq!(rollback(&mut operations).len(), 1);
    assert_eq!(operations[0].location, Location::Staged);
    assert_eq!(fs::read_to_string(&source).unwrap(), "external");
    assert_eq!(fs::read_to_string(&staged).unwrap(), "original");
}

#[test]
fn rejects_nul_paths_without_moving_the_source() {
    let sandbox = Sandbox::new();
    let source = sandbox.0.join("source");
    let destination = sandbox.0.join("destination");
    fs::write(&source, "safe").unwrap();
    assert!(parse_name("destination\0ignored").is_err());
    assert!(move_without_overwrite(&source, &sandbox.0.join("destination\0ignored")).is_err());
    assert!(move_without_overwrite(&sandbox.0.join("source\0ignored"), &destination).is_err());
    assert_eq!(fs::read_to_string(source).unwrap(), "safe");
    assert!(!destination.exists());
}

#[cfg(unix)]
#[test]
fn unix_filename_validation_remains_case_sensitive_and_permissive() {
    for name in [
        "CON",
        "nul.txt",
        "name:stream",
        "back\\slash",
        "trailing.",
        "trailing ",
        ".ROCKDOWN-TRASH",
        ".rockdown-stage-old",
    ] {
        assert!(parse_name(name).is_ok(), "rejected {name:?}");
    }
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join("a"), "A").unwrap();
    fs::write(sandbox.0.join("b"), "B").unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    rename_line(&mut explorer, "b", "A");
    assert!(explorer.desired().is_ok());
}

#[cfg(windows)]
#[test]
fn windows_rejects_invalid_and_reserved_names_before_mutation() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join("original"), "safe").unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    for name in [
        "CON",
        "con.txt",
        "PrN.log",
        "AUX",
        "nul.tar.gz",
        "COM1",
        "lpt9.txt",
        "COM\u{b9}.txt",
        "LPT\u{b2}",
        "COM\u{b3}",
        "CONIN$",
        "conout$.txt",
        "CLOCK$",
        "CON .txt",
        "name:stream",
        "C:relative",
        "back\\slash",
        "a<b",
        "a>b",
        "a\"b",
        "a|b",
        "a?b",
        "a*b",
        "control\u{1f}",
        "trailing.",
        "trailing ",
        ".ROCKDOWN-TRASH",
        ".Rockdown-Stage",
        ".ROCKDOWN-STAGE-recovery",
    ] {
        for text in [name.to_owned(), format!("{name}/")] {
            assert!(parse_name(&text).is_err(), "accepted {text:?}");
        }
        explorer.buffer.lines[0].text = name.to_owned();
        assert!(explorer.commit().is_err(), "committed {name:?}");
        assert_eq!(
            fs::read_to_string(sandbox.0.join("original")).unwrap(),
            "safe"
        );
        assert_eq!(fs::read_dir(&sandbox.0).unwrap().count(), 1);
    }
    assert!(parse_name(&"x".repeat(256)).is_err());
    assert!(parse_name(&"\u{1f600}".repeat(128)).is_err());
    for name in [
        "COM0",
        "COM10",
        "LPT0",
        "console.txt",
        "auxiliary",
        "normal name.txt",
        "\u{e9}.txt",
    ] {
        assert!(parse_name(name).is_ok(), "rejected {name:?}");
    }
}

#[cfg(windows)]
#[test]
fn windows_rejects_case_insensitive_destinations_before_mutation() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join("first"), "one").unwrap();
    fs::write(sandbox.0.join("second"), "two").unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    for (left, right) in [
        ("same", "SAME"),
        ("\u{e9}.txt", "\u{c9}.txt"),
        ("first", "FIRST"),
    ] {
        explorer.buffer.lines[0].text = left.to_owned();
        explorer.buffer.lines[1].text = right.to_owned();
        let error = explorer.commit().unwrap_err();
        assert!(error.to_string().contains("Duplicate explorer destination"));
        assert_eq!(fs::read_to_string(sandbox.0.join("first")).unwrap(), "one");
        assert_eq!(fs::read_to_string(sandbox.0.join("second")).unwrap(), "two");
        assert_eq!(fs::read_dir(&sandbox.0).unwrap().count(), 2);
    }
}

#[cfg(windows)]
#[test]
fn windows_no_replace_preserves_a_case_variant_destination() {
    let sandbox = Sandbox::new();
    let source = sandbox.0.join("source");
    fs::write(&source, "original").unwrap();
    fs::write(sandbox.0.join("DESTINATION"), "external").unwrap();
    assert!(move_without_overwrite(&source, &sandbox.0.join("destination")).is_err());
    assert_eq!(fs::read_to_string(&source).unwrap(), "original");
    assert_eq!(
        fs::read_to_string(sandbox.0.join("DESTINATION")).unwrap(),
        "external"
    );
}

#[cfg(windows)]
#[test]
fn windows_hides_and_protects_case_variants_of_recovery_names() {
    let sandbox = Sandbox::new();
    fs::create_dir(sandbox.0.join(".ROCKDOWN-TRASH")).unwrap();
    fs::create_dir(sandbox.0.join(".Rockdown-Stage-recovery")).unwrap();
    fs::write(sandbox.0.join(".Rockdown-Stage-recovery/precious"), "safe").unwrap();
    fs::write(sandbox.0.join("visible"), "delete me").unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    assert_eq!(explorer.buffer.text(), "visible");
    explorer.buffer.lines[0].text.clear();
    explorer.commit().unwrap();
    assert_eq!(
        fs::read_to_string(sandbox.0.join(".Rockdown-Stage-recovery/precious")).unwrap(),
        "safe"
    );
    assert_eq!(explorer.buffer.text(), "");
}

#[cfg(windows)]
#[test]
fn windows_paths_preserve_utf16_and_unicode_renames() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    let raw = OsString::from_wide(&[b'x' as u16, 0xd800]);
    assert_eq!(
        windows_path(Path::new(&raw)).unwrap(),
        [b'x' as u16, 0xd800, 0]
    );
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join("\u{e9}-\u{1f600}"), "contents").unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    rename_line(&mut explorer, "\u{e9}-\u{1f600}", "\u{c9}-\u{1f600}");
    explorer.commit().unwrap();
    assert_eq!(explorer.buffer.text(), "\u{c9}-\u{1f600}");
    assert_eq!(
        fs::read_to_string(sandbox.0.join("\u{c9}-\u{1f600}")).unwrap(),
        "contents"
    );
}

#[cfg(windows)]
#[test]
fn windows_fingerprint_detects_same_length_and_mtime_replacement() {
    let sandbox = Sandbox::new();
    let retained = Sandbox::new();
    let path = sandbox.0.join("original");
    fs::write(&path, "before").unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    let before = explorer.snapshot["original"].fingerprint.clone();
    fs::rename(&path, retained.0.join("original")).unwrap();
    fs::write(&path, "after!").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(before.modified.unwrap()))
        .unwrap();
    let mut after = Fingerprint::read(&path).unwrap();
    assert_eq!(before.len, after.len);
    assert_eq!(before.modified, after.modified);
    assert_ne!(before.identity, after.identity);
    // Identity still detects replacement if creation timestamps happen to match.
    after.creation_time = before.creation_time;
    assert!(!same_restored_entry(&before, &after));
    explorer.adopt_rolled_back_snapshot();
    assert_eq!(explorer.snapshot["original"].fingerprint, before);
    assert!(explorer.enter().is_err());
    rename_line(&mut explorer, "original", "renamed");
    assert!(explorer.commit().is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "after!");
    assert!(!sandbox.0.join("renamed").exists());
}

#[cfg(windows)]
#[test]
fn windows_reparse_fingerprints_and_trash_never_follow_targets() {
    let sandbox = Sandbox::new();
    let target = Sandbox::new();
    let link = sandbox.0.join(".ROCKDOWN-TRASH");
    if let Err(error) = std::os::windows::fs::symlink_dir(&target.0, &link) {
        if error.raw_os_error() == Some(1314) {
            eprintln!("Skipping symlink test: enable Windows Developer Mode or symlink privilege");
            return;
        }
        panic!("Cannot create test symlink: {error}");
    }
    let before = Fingerprint::read(&link).unwrap();
    assert_eq!(before.kind, Kind::Symlink);
    fs::write(target.0.join("precious"), "safe").unwrap();
    assert_eq!(Fingerprint::read(&link).unwrap(), before);
    assert!(ensure_trash_directory(&link).is_err());
    fs::write(sandbox.0.join("original"), "safe").unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    explorer.buffer.lines[0].text.clear();
    assert!(explorer.commit().is_err());
    assert_eq!(
        fs::read_to_string(sandbox.0.join("original")).unwrap(),
        "safe"
    );
    assert_eq!(
        fs::read_to_string(target.0.join("precious")).unwrap(),
        "safe"
    );
}

#[test]
fn failed_install_rolls_back_a_partially_completed_swap() {
    let sandbox = Sandbox::new();
    let staging = unique_directory(&sandbox.0, ".stage").unwrap();
    fs::write(sandbox.0.join("a"), "A").unwrap();
    fs::write(sandbox.0.join("b"), "B").unwrap();
    let before = Fingerprint::read(&sandbox.0.join("a")).unwrap();
    fs::rename(sandbox.0.join("a"), staging.join("0")).unwrap();
    fs::rename(sandbox.0.join("b"), staging.join("1")).unwrap();
    fs::rename(staging.join("0"), sandbox.0.join("b")).unwrap();
    let mut operations = vec![
        Operation {
            source: Some(sandbox.0.join("a")),
            staged: staging.join("0"),
            destination: sandbox.0.join("b"),
            directory: false,
            location: Location::Destination,
        },
        Operation {
            source: Some(sandbox.0.join("b")),
            staged: staging.join("1"),
            destination: sandbox.0.join("a"),
            directory: false,
            location: Location::Staged,
        },
    ];
    assert!(rollback(&mut operations).is_empty());
    assert_eq!(fs::read_to_string(sandbox.0.join("a")).unwrap(), "A");
    assert_eq!(fs::read_to_string(sandbox.0.join("b")).unwrap(), "B");
    assert!(same_restored_entry(
        &before,
        &Fingerprint::read(&sandbox.0.join("a")).unwrap()
    ));
}

#[test]
fn replacing_a_line_with_identical_text_is_still_a_pending_filesystem_edit() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join("original"), "preserve until committed").unwrap();
    let mut explorer = Explorer::open(&sandbox.0).unwrap();
    for key in ["y", "y", "p", "k", "d", "d"] {
        explorer.buffer.key(key);
    }
    assert_eq!(explorer.buffer.text(), "original");
    assert!(explorer.parent().is_err());
    assert!(explorer.reload().is_err());
    assert_eq!(
        fs::read_to_string(sandbox.0.join("original")).unwrap(),
        "preserve until committed"
    );
}
