use envlume_workspace::{discover, entry_path};
use std::{fs, path::Path};

#[test]
fn gitfile_is_bounded_metadata_and_only_the_admin_layout_suggests_a_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join(".git");
    fs::write(&marker, "gitdir: /unopened/worktrees/branch\n").unwrap();
    let first = discover(tmp.path()).unwrap();
    assert_eq!(first.git_kind, "worktree");
    fs::write(
        &marker,
        "gitdir: /unopened/worktrees/repo/.git/modules/submodule\n",
    )
    .unwrap();
    let second = discover(tmp.path()).unwrap();
    assert_ne!(first.fingerprint, second.fingerprint);
    assert_eq!(second.git_kind, "gitfile");
    fs::write(marker, vec![b'x'; 9000]).unwrap();
    assert!(discover(tmp.path()).is_err());
}
#[test]
fn lexical_parent_and_missing_directory_are_rejected() {
    assert!(entry_path(Path::new("a/../b")).is_err());
    let temp = tempfile::tempdir().unwrap();
    assert!(discover(&temp.path().join("missing")).is_err());
}
#[cfg(windows)]
#[test]
fn unsafe_windows_prefixes_and_ambiguous_dos_paths_fail_before_io() {
    for path in [
        r"\\host.invalid\share\workspace",
        r"\\?\UNC\host.invalid\share",
        r"\\.\PhysicalDrive0",
        r"C:relative",
        r"C:\example\.. \outside",
        r"C:\example\NUL.txt",
        r"C:\example\file:stream",
    ] {
        assert!(entry_path(Path::new(path)).is_err(), "{path}");
    }
}
