use super::*;

fn file(path: &str, size: u64, hash: &str) -> CleanupFile {
    CleanupFile {
        path: path.to_string(),
        size,
        hash: Some(hash.to_string()),
        mtime: 100,
        in_sync: true,
    }
}

fn analyse(mut files: Vec<CleanupFile>, hidden: &[&str]) -> CleanupView {
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let hidden: Vec<String> = hidden.iter().map(|path| path.to_string()).collect();
    analyse_cleanup(&files, &hidden)
}

fn summary(view: &CleanupView) -> Vec<(String, String, usize)> {
    view.duplicates
        .iter()
        .map(|suggestion| {
            (
                suggestion.kind.clone(),
                suggestion.path.clone(),
                suggestion.file_count,
            )
        })
        .collect()
}

/// Every content keeps at least one copy that no suggestion deletes.
fn assert_one_copy_stays(files: &[CleanupFile], view: &CleanupView) {
    let deleted: HashSet<&str> = view
        .duplicates
        .iter()
        .flat_map(|suggestion| suggestion.all_pairs.iter().map(|pair| pair.path.as_str()))
        .collect();
    for pair in view
        .duplicates
        .iter()
        .flat_map(|suggestion| &suggestion.all_pairs)
    {
        assert!(!deleted.contains(pair.kept_path.as_str()), "{pair:?}");
    }
    for file in files {
        let hash = file.hash.as_deref();
        let has_copy = files.iter().any(|other| other.hash.as_deref() == hash);
        assert!(has_copy);
    }
    let kept_hashes: HashSet<Option<&str>> = files
        .iter()
        .filter(|file| !deleted.contains(file.path.as_str()))
        .map(|file| file.hash.as_deref())
        .collect();
    for file in files {
        assert!(
            kept_hashes.contains(&file.hash.as_deref()),
            "{} lost",
            file.path
        );
    }
}

#[test]
fn an_older_snapshot_that_differs_in_a_few_files_is_one_suggestion() {
    let mut files = Vec::new();
    for snapshot in ["Games/Backup/20250223", "Games/Backup/20250809"] {
        for number in 0..10 {
            files.push(file(
                &format!("{snapshot}/ROM/game{number}.gba"),
                1000,
                &format!("rom{number}"),
            ));
        }
    }
    files.push(file("Games/Backup/20250223/SAVER/a.sav", 10, "save-old"));
    files.push(file("Games/Backup/20250809/SAVER/a.sav", 10, "save-new"));
    let view = analyse(files.clone(), &[]);

    assert_eq!(
        summary(&view),
        vec![(
            "partial".to_string(),
            "Games/Backup/20250223".to_string(),
            10
        )]
    );
    let suggestion = &view.duplicates[0];
    assert_eq!(
        suggestion.folder_file_count, 11,
        "one file differs and stays"
    );
    assert_eq!(suggestion.delete_paths, vec!["Games/Backup/20250223/ROM"]);
    assert_eq!(
        suggestion.kept_in,
        vec![KeptLocation {
            path: "Games/Backup/20250809".to_string(),
            file_count: 10
        }]
    );
    assert_eq!(view.duplicate_bytes, 10_000);
    assert_one_copy_stays(&files, &view);
}

#[test]
fn a_folder_of_copies_is_suggested_whole_at_its_highest_level() {
    let files = vec![
        file("Pictures/Archive/2017/10/a.jpg", 5, "a"),
        file("Pictures/Archive/2017/10/b.jpg", 6, "b"),
        file("Pictures/Archive/2017/10/unique.jpg", 7, "u"),
        file("Pictures/duplicates/Phone/a.jpg", 5, "a"),
        file("Pictures/duplicates/Phone/sub/b.jpg", 6, "b"),
    ];
    let view = analyse(files.clone(), &[]);
    assert_eq!(
        summary(&view),
        vec![("folder".to_string(), "Pictures/duplicates".to_string(), 2)]
    );
    let pairs: Vec<(&str, &str)> = view.duplicates[0]
        .pairs
        .iter()
        .map(|pair| (pair.path.as_str(), pair.kept_path.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            (
                "Pictures/duplicates/Phone/sub/b.jpg",
                "Pictures/Archive/2017/10/b.jpg"
            ),
            (
                "Pictures/duplicates/Phone/a.jpg",
                "Pictures/Archive/2017/10/a.jpg"
            ),
        ]
    );
    assert_one_copy_stays(&files, &view);
}

#[test]
fn of_two_identical_folders_only_one_is_suggested() {
    let files = vec![
        file("Snap1/a", 5, "a"),
        file("Snap1/b", 6, "b"),
        file("Snap2/a", 5, "a"),
        file("Snap2/b", 6, "b"),
    ];
    let view = analyse(files.clone(), &[]);
    assert_eq!(
        summary(&view),
        vec![("folder".to_string(), "Snap1".to_string(), 2)]
    );
    assert_one_copy_stays(&files, &view);
}

#[test]
fn a_folder_named_old_or_copy_is_the_one_suggested() {
    let files = vec![file("Keep/a", 5, "a"), file("Old Stuff/a", 5, "a")];
    let view = analyse(files, &[]);
    assert_eq!(
        summary(&view),
        vec![("folder".to_string(), "Old Stuff".to_string(), 1)]
    );
}

#[test]
fn the_copy_filed_by_date_is_the_one_kept() {
    // The single-file Archive folders would be suggested whole before the
    // album's loose files if dates were not preferred.
    let files = vec![
        file("Pictures/Albums/Red/a.jpg", 5, "a"),
        file("Pictures/Albums/Red/b.jpg", 6, "b"),
        file("Pictures/Albums/Red/c.jpg", 7, "c"),
        file("Pictures/Albums/Red/unique.jpg", 8, "u"),
        file("Pictures/Archive/2019/02/a.jpg", 5, "a"),
        file("Pictures/Archive/2022/unknown/b.jpg", 6, "b"),
        file("Pictures/Archive/2010/c.jpg", 7, "c"),
    ];
    let view = analyse(files.clone(), &[]);
    assert_eq!(
        summary(&view),
        vec![("files".to_string(), "Pictures/Albums/Red".to_string(), 3)]
    );
    assert_one_copy_stays(&files, &view);
}

#[test]
fn identical_content_under_another_name_is_not_a_copy() {
    let files = vec![
        file("Cheats/2200/game-a.cht", 5, "same"),
        file("Cheats/2400/game-b.cht", 5, "same"),
    ];
    let view = analyse(files, &[]);
    assert!(view.duplicates.is_empty(), "{:?}", summary(&view));
}

#[test]
fn loose_copies_are_grouped_by_their_folder() {
    let files = vec![
        file("Albums/Red/x.jpg", 5, "x"),
        file("Albums/Red/y.jpg", 6, "y"),
        file("Albums/Red/own.jpg", 7, "own"),
        file("Albums/Red/own2.jpg", 8, "own2"),
        file("Archive/x.jpg", 5, "x"),
        file("Archive/y.jpg", 6, "y"),
        file("Archive/z.jpg", 9, "z"),
    ];
    let view = analyse(files.clone(), &[]);
    assert_eq!(
        summary(&view),
        vec![("files".to_string(), "Albums/Red".to_string(), 2)]
    );
    assert_eq!(view.duplicates[0].folder_file_count, 4);
    assert_one_copy_stays(&files, &view);
}

#[test]
fn a_hidden_folder_is_neither_suggested_nor_a_reason_to_suggest_its_originals() {
    let files = vec![
        file("Albums/Red/x.jpg", 5, "x"),
        file("Archive/x.jpg", 5, "x"),
        file("Archive/other.jpg", 9, "other"),
    ];
    let view = analyse(files, &["Albums/Red"]);
    assert!(view.duplicates.is_empty(), "{:?}", summary(&view));
    assert_eq!(view.hidden, vec!["Albums/Red".to_string()]);
}

#[test]
fn a_file_not_in_sync_stops_its_folder_being_suggested_whole() {
    let mut pending = file("Copy/new.txt", 3, "n");
    pending.in_sync = false;
    let files = vec![
        file("Copy/a", 5, "a"),
        file("Copy/b", 6, "b"),
        pending,
        file("Orig/a", 5, "a"),
        file("Orig/b", 6, "b"),
    ];
    let view = analyse(files, &[]);
    assert_eq!(
        summary(&view),
        vec![("files".to_string(), "Copy".to_string(), 2)]
    );
    assert_eq!(view.duplicates[0].delete_paths, vec!["Copy/a", "Copy/b"]);
}

#[test]
fn large_files_are_listed_biggest_first_with_their_copies() {
    let big = LARGE_FILE_MIN_BYTES;
    let files = vec![
        file("Docs/linux.iso", big * 20, "iso"),
        file("Videos/a.mp4", big * 2, "mp4"),
        file("Videos/copy/a.mp4", big * 2, "mp4"),
        file("Docs/small.txt", 10, "small"),
    ];
    let view = analyse(files, &[]);
    let large: Vec<(&str, usize)> = view
        .large_files
        .iter()
        .map(|file| (file.path.as_str(), file.copies.len()))
        .collect();
    assert_eq!(
        large,
        vec![
            ("Docs/linux.iso", 0),
            ("Videos/a.mp4", 1),
            ("Videos/copy/a.mp4", 1)
        ]
    );
}

#[test]
fn byte_comparison_catches_a_difference_and_stray_files() {
    let root = std::env::temp_dir().join(format!(
        "somedrive-cleanup-test-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(root.join("Copy")).expect("dirs");
    std::fs::create_dir_all(root.join("Orig")).expect("dirs");
    std::fs::write(root.join("Copy/a"), b"same").expect("write");
    std::fs::write(root.join("Orig/a"), b"same").expect("write");
    std::fs::write(root.join("Copy/b"), b"one!").expect("write");
    std::fs::write(root.join("Orig/b"), b"two!").expect("write");
    let pair = |path: &str, kept: &str| DuplicatePair {
        path: path.to_string(),
        kept_path: kept.to_string(),
        size: 4,
    };

    verify_identical_on_disk(&root, &[pair("Copy/a", "Orig/a")]).expect("identical");
    let error =
        verify_identical_on_disk(&root, &[pair("Copy/a", "Orig/a"), pair("Copy/b", "Orig/b")])
            .expect_err("differs");
    assert!(error.contains("Copy/b"), "{error}");

    std::fs::remove_dir_all(&root).expect("clean up");
}
