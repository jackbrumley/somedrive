use super::*;

// Turns the accepted deletions into the few suggestions the Clean up tab
// shows, each with the identical copies that stay, and lists large files.

impl Analysis<'_> {
    /// One suggestion per whole folder, per folder that is nearly all
    /// copies, and per folder holding loose copies; biggest first.
    pub(crate) fn suggestions(&self, accepted: &[Unit]) -> Vec<DuplicateSuggestion> {
        let mut deleted_files: HashMap<&str, usize> = HashMap::new();
        let mut deleted_bytes: HashMap<&str, u64> = HashMap::new();
        let mut total_bytes: HashMap<&str, u64> = HashMap::new();
        for (index, file) in self.files.iter().enumerate() {
            for folder in ancestors(&file.path) {
                *total_bytes.entry(folder).or_default() += file.size;
                if self.deleted[index] {
                    *deleted_files.entry(folder).or_default() += 1;
                    *deleted_bytes.entry(folder).or_default() += file.size;
                }
            }
        }
        let tally = FolderTally {
            deleted_files,
            deleted_bytes,
            total_bytes,
        };
        let mut suggestions = Vec::new();
        self.collect_suggestions("", accepted, &tally, &mut suggestions);
        suggestions.sort_by(|left, right| {
            right
                .bytes
                .cmp(&left.bytes)
                .then_with(|| left.path.cmp(&right.path))
        });
        suggestions
    }

    fn collect_suggestions(
        &self,
        folder: &str,
        accepted: &[Unit],
        tally: &FolderTally,
        suggestions: &mut Vec<DuplicateSuggestion>,
    ) {
        for subfolder in self.subfolders.get(folder).into_iter().flatten() {
            let deleted = tally
                .deleted_files
                .get(subfolder.as_str())
                .copied()
                .unwrap_or(0);
            if deleted == 0 {
                continue;
            }
            let whole = accepted
                .iter()
                .any(|unit| matches!(unit, Unit::Folder(path) if path == subfolder));
            let total = self.total_files.get(subfolder).copied().unwrap_or(0);
            let bytes = tally
                .deleted_bytes
                .get(subfolder.as_str())
                .copied()
                .unwrap_or(0);
            let all_bytes = tally
                .total_bytes
                .get(subfolder.as_str())
                .copied()
                .unwrap_or(0);
            let mostly_copies = deleted as u64 * 10 >= total as u64 * MOSTLY_COPIES_TENTHS
                && bytes * 10 >= all_bytes * MOSTLY_COPIES_TENTHS;
            if whole {
                suggestions.push(self.suggestion("folder", subfolder, accepted));
            } else if mostly_copies {
                suggestions.push(self.suggestion("partial", subfolder, accepted));
            } else {
                self.collect_suggestions(subfolder, accepted, tally, suggestions);
            }
        }
        let loose = accepted.iter().any(|unit| {
            matches!(unit, Unit::File(index) if parent_folder(&self.files[*index].path) == folder)
        });
        if loose {
            suggestions.push(self.suggestion("files", folder, accepted));
        }
    }

    fn suggestion(&self, kind: &str, folder: &str, accepted: &[Unit]) -> DuplicateSuggestion {
        let units: Vec<&Unit> = accepted
            .iter()
            .filter(|unit| match (kind, unit) {
                ("files", Unit::File(index)) => parent_folder(&self.files[*index].path) == folder,
                ("files", Unit::Folder(_)) => false,
                (_, Unit::Folder(path)) => path == folder || is_inside(path, folder),
                (_, Unit::File(index)) => is_inside(&self.files[*index].path, folder),
            })
            .collect();
        let mut delete_paths = Vec::new();
        let mut members = Vec::new();
        for unit in units {
            match unit {
                Unit::Folder(path) => {
                    delete_paths.push(path.clone());
                    members.extend(self.files_in(path));
                }
                Unit::File(index) => {
                    delete_paths.push(self.files[*index].path.clone());
                    members.push(*index);
                }
            }
        }
        delete_paths.sort();

        let mut pairs = Vec::new();
        let mut kept_counts: HashMap<String, usize> = HashMap::new();
        for &member in &members {
            let file = &self.files[member];
            let relative = if folder.is_empty() {
                file.path.as_str()
            } else {
                &file.path[folder.len() + 1..]
            };
            let kept = self.kept_copy(member, relative);
            let kept_root = if kept == relative {
                String::new()
            } else if let Some(root) = kept
                .strip_suffix(relative)
                .and_then(|rest| rest.strip_suffix('/'))
            {
                root.to_string()
            } else {
                parent_folder(kept).to_string()
            };
            *kept_counts.entry(kept_root).or_default() += 1;
            pairs.push(DuplicatePair {
                path: file.path.clone(),
                kept_path: kept.to_string(),
                size: file.size,
            });
        }
        let mut kept_in: Vec<KeptLocation> = kept_counts
            .into_iter()
            .map(|(path, file_count)| KeptLocation { path, file_count })
            .collect();
        kept_in.sort_by(|left, right| {
            right
                .file_count
                .cmp(&left.file_count)
                .then_with(|| left.path.cmp(&right.path))
        });
        let kept_in_more = kept_in.len().saturating_sub(KEPT_LOCATION_LIMIT);
        kept_in.truncate(KEPT_LOCATION_LIMIT);

        let file_count = pairs.len();
        let bytes = pairs.iter().map(|pair| pair.size).sum();
        pairs.sort_by(|left, right| {
            right
                .size
                .cmp(&left.size)
                .then_with(|| left.path.cmp(&right.path))
        });
        let all_pairs = pairs.clone();
        pairs.truncate(SUGGESTION_PAIR_LIMIT);
        let folder_file_count = if kind == "files" {
            self.direct_files.get(folder).map_or(0, Vec::len)
        } else {
            self.total_files.get(folder).copied().unwrap_or(0)
        };
        DuplicateSuggestion {
            id: format!("{kind}:{file_count}:{bytes}:{folder}"),
            kind: kind.to_string(),
            path: folder.to_string(),
            delete_paths,
            file_count,
            bytes,
            folder_file_count,
            kept_in,
            kept_in_more,
            pairs,
            all_pairs,
        }
    }

    /// The identical copy that stays for a file being deleted: preferably
    /// at the same place in the other folder (`relative` is the file's path
    /// within the suggestion), else one with the same name.
    fn kept_copy(&self, index: usize, relative: &str) -> &str {
        let name = &self.files[index].path[parent_folder(&self.files[index].path).len()..];
        let mirrored = format!("/{relative}");
        let mut staying: Vec<&str> = self.copies[index]
            .iter()
            .filter(|&&other| !self.deleted[other])
            .map(|&other| self.files[other].path.as_str())
            .collect();
        staying.sort_by_key(|path| {
            (
                !(path.ends_with(&mirrored) || *path == relative),
                !path.ends_with(name),
                path.len(),
                *path,
            )
        });
        staying.first().copied().unwrap_or_default()
    }

    /// The biggest files, with any identical copies.
    pub(crate) fn large_files(&self) -> Vec<LargeFile> {
        let mut large: Vec<usize> = (0..self.files.len())
            .filter(|&index| {
                let file = &self.files[index];
                file.in_sync && !self.locked[index] && file.size >= LARGE_FILE_MIN_BYTES
            })
            .collect();
        large.sort_by(|&left, &right| {
            self.files[right]
                .size
                .cmp(&self.files[left].size)
                .then_with(|| self.files[left].path.cmp(&self.files[right].path))
        });
        large.truncate(LARGE_FILE_LIMIT);
        large
            .into_iter()
            .map(|index| {
                let file = &self.files[index];
                let mut copies: Vec<String> = self.copies[index]
                    .iter()
                    .map(|&other| self.files[other].path.clone())
                    .collect();
                copies.sort();
                LargeFile {
                    path: file.path.clone(),
                    size: file.size,
                    modified: file.mtime,
                    copies,
                }
            })
            .collect()
    }
}

pub(crate) struct FolderTally<'a> {
    deleted_files: HashMap<&'a str, usize>,
    deleted_bytes: HashMap<&'a str, u64>,
    total_bytes: HashMap<&'a str, u64>,
}
