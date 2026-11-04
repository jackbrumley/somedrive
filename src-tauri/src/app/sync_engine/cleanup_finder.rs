use super::*;

// The Clean up tab's analysis: identical files (same name, and the same
// OneDrive hash and size) and very large files, worked out from the sync
// database alone.
//
// Duplicates are suggested as high up the folder tree as possible: a whole
// folder when everything in it has an identical copy elsewhere, otherwise
// the identical part of a folder that is nearly all copies, otherwise the
// copies one folder at a time. One copy of every file always stays, even
// if every suggestion is deleted.

/// Files at least this big are listed under "Large files".
pub(crate) const LARGE_FILE_MIN_BYTES: u64 = 100 * 1024 * 1024;
pub(crate) const LARGE_FILE_LIMIT: usize = 50;
/// A suggestion lists at most this many of its files (the UI renders them).
pub(crate) const SUGGESTION_PAIR_LIMIT: usize = 50;
/// A folder is offered as one suggestion when at least this share (in
/// tenths) of its files and of its bytes can go.
pub(crate) const MOSTLY_COPIES_TENTHS: u64 = 9;
pub(crate) const KEPT_LOCATION_LIMIT: usize = 3;
/// Words in a folder or file name that mark the copy to delete first.
const DELETE_FIRST_WORDS: [&str; 5] = ["duplicate", "duplicates", "copy", "copies", "old"];

/// One tracked file, as the analysis sees it.
#[derive(Debug, Clone)]
pub(crate) struct CleanupFile {
    pub(crate) path: String,
    pub(crate) size: u64,
    pub(crate) hash: Option<String>,
    pub(crate) mtime: i64,
    /// In sync on both sides with a known hash: may be suggested for
    /// deletion or kept as the copy. Other tracked files (pending changes,
    /// skipped items) only stop their folders being suggested whole.
    pub(crate) in_sync: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CleanupView {
    pub duplicates: Vec<DuplicateSuggestion>,
    pub large_files: Vec<LargeFile>,
    /// What deleting every duplicate suggestion would free.
    pub duplicate_bytes: u64,
    /// Paths the user hid from Clean up: neither suggested nor counted as
    /// a copy of anything.
    pub hidden: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateSuggestion {
    /// Stable while the suggestion is unchanged; the delete names it.
    pub id: String,
    /// `folder` (the whole folder), `partial` (the identical part of a
    /// folder) or `files` (identical files directly in a folder).
    pub kind: String,
    /// The folder the suggestion is about ("" is the top of OneDrive).
    pub path: String,
    /// What gets deleted: whole folders and single files.
    pub delete_paths: Vec<String>,
    pub file_count: usize,
    pub bytes: u64,
    /// All files in the folder (for `partial`: the ones that stay differ).
    pub folder_file_count: usize,
    /// Where the identical copies that stay are, most files first.
    pub kept_in: Vec<KeptLocation>,
    pub kept_in_more: usize,
    /// Each file to delete with the identical copy that stays, biggest
    /// first; at most `SUGGESTION_PAIR_LIMIT`.
    pub pairs: Vec<DuplicatePair>,
    /// Every pair, checked byte for byte before a delete.
    #[serde(skip)]
    pub(crate) all_pairs: Vec<DuplicatePair>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeptLocation {
    pub path: String,
    pub file_count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DuplicatePair {
    pub path: String,
    pub kept_path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    pub path: String,
    pub size: u64,
    pub modified: i64,
    /// Other files with identical content, if any.
    pub copies: Vec<String>,
}

/// A piece the greedy selection accepted: a whole folder or one file.
#[derive(Debug, Clone)]
pub(crate) enum Unit {
    Folder(String),
    File(usize),
}

pub(crate) struct Analysis<'a> {
    pub(crate) files: &'a [CleanupFile],
    /// For each file: the other files with identical content.
    pub(crate) copies: Vec<Vec<usize>>,
    /// Files the user hid (or inside a hidden folder): left out of the
    /// matching, so they are neither suggested nor the reason another copy
    /// is (hiding an album must not suggest deleting its originals).
    pub(crate) locked: Vec<bool>,
    pub(crate) subfolders: HashMap<String, Vec<String>>,
    pub(crate) direct_files: HashMap<String, Vec<usize>>,
    /// Per folder: every tracked file inside, and how many could go.
    pub(crate) total_files: HashMap<String, usize>,
    pub(crate) covered_files: HashMap<String, usize>,
    pub(crate) newest_mtime: HashMap<String, i64>,
    pub(crate) deleted: Vec<bool>,
}

/// Builds the Clean up view. `files` must be sorted by path.
pub(crate) fn analyse_cleanup(files: &[CleanupFile], hidden: &[String]) -> CleanupView {
    let mut analysis = Analysis::new(files, hidden);
    let accepted = analysis.select_deletions();
    let duplicates = analysis.suggestions(&accepted);
    let duplicate_bytes = duplicates.iter().map(|suggestion| suggestion.bytes).sum();
    CleanupView {
        large_files: analysis.large_files(),
        duplicates,
        duplicate_bytes,
        hidden: hidden.to_vec(),
    }
}

pub(crate) fn file_name(path: &str) -> &str {
    path.rfind('/')
        .map(|index| &path[index + 1..])
        .unwrap_or(path)
}

pub(crate) fn parent_folder(path: &str) -> &str {
    path.rfind('/').map(|index| &path[..index]).unwrap_or("")
}

/// Every folder above `path`, top first (the top of OneDrive excluded).
pub(crate) fn ancestors(path: &str) -> Vec<&str> {
    path.match_indices('/')
        .map(|(index, _)| &path[..index])
        .collect()
}

fn is_at_or_inside(path: &str, folder: &str) -> bool {
    path == folder || is_inside(path, folder)
}

fn delete_first_rank(path: &str) -> u8 {
    let marked = path.split('/').any(|segment| {
        segment
            .to_lowercase()
            .split(|character: char| !character.is_alphanumeric())
            .any(|word| DELETE_FIRST_WORDS.contains(&word))
    });
    if marked {
        0
    } else {
        1
    }
}

impl<'a> Analysis<'a> {
    fn new(files: &'a [CleanupFile], hidden: &[String]) -> Self {
        let locked: Vec<bool> = files
            .iter()
            .map(|file| hidden.iter().any(|path| is_at_or_inside(&file.path, path)))
            .collect();
        // A copy has the same content and the same name: identical content
        // under another name is often meaningful (a game's cheat file, a
        // cache entry), so it is not offered for deletion.
        let mut groups: HashMap<(&str, u64, String), Vec<usize>> = HashMap::new();
        for (index, file) in files.iter().enumerate() {
            let matchable = file.in_sync && file.size > 0 && !locked[index];
            if let (true, Some(hash)) = (matchable, file.hash.as_deref()) {
                let name = file_name(&file.path).to_lowercase();
                groups
                    .entry((hash, file.size, name))
                    .or_default()
                    .push(index);
            }
        }
        let mut copies = vec![Vec::new(); files.len()];
        for members in groups.values().filter(|members| members.len() > 1) {
            for &member in members {
                copies[member] = members
                    .iter()
                    .copied()
                    .filter(|&other| other != member)
                    .collect();
            }
        }

        let mut subfolders: HashMap<String, Vec<String>> = HashMap::new();
        let mut direct_files: HashMap<String, Vec<usize>> = HashMap::new();
        let mut total_files: HashMap<String, usize> = HashMap::new();
        let mut covered_files: HashMap<String, usize> = HashMap::new();
        let mut newest_mtime: HashMap<String, i64> = HashMap::new();
        for (index, file) in files.iter().enumerate() {
            direct_files
                .entry(parent_folder(&file.path).to_string())
                .or_default()
                .push(index);
            // A folder has a copy outside it when some copy shares fewer
            // folders with this file than the folder's depth.
            let fewest_shared = copies[index]
                .iter()
                .map(|&other| common_folder_depth(&file.path, &files[other].path))
                .min();
            for (depth, folder) in ancestors(&file.path).into_iter().enumerate() {
                let parent = parent_folder(folder);
                // Sorted paths keep each folder's files together.
                let siblings = subfolders.entry(parent.to_string()).or_default();
                if siblings.last().map(String::as_str) != Some(folder) {
                    siblings.push(folder.to_string());
                }
                *total_files.entry(folder.to_string()).or_default() += 1;
                let newest = newest_mtime.entry(folder.to_string()).or_insert(i64::MIN);
                *newest = (*newest).max(file.mtime);
                let copy_outside = fewest_shared.is_some_and(|shared| shared < depth + 1);
                if copy_outside {
                    *covered_files.entry(folder.to_string()).or_default() += 1;
                }
            }
        }
        Analysis {
            files,
            copies,
            locked,
            subfolders,
            direct_files,
            total_files,
            covered_files,
            newest_mtime,
            deleted: vec![false; files.len()],
        }
    }

    /// Whether everything in `folder` has an identical copy outside it.
    fn covered(&self, folder: &str) -> bool {
        let total = self.total_files.get(folder).copied().unwrap_or(0);
        total > 0 && self.covered_files.get(folder).copied().unwrap_or(0) == total
    }

    /// The order pieces are offered in; the first of two copies is the one
    /// suggested. Folders named "old", "copy" or "duplicates" go first, then
    /// whole folders (the older of two snapshots first), then single files
    /// by folder, so copies shared by two folders are always taken from the
    /// same one of them.
    fn unit_key(&self, unit: &Unit) -> (u8, u8, i64, String) {
        match unit {
            Unit::Folder(path) => (
                delete_first_rank(path),
                0,
                self.newest_mtime.get(path).copied().unwrap_or(0),
                path.clone(),
            ),
            Unit::File(index) => {
                let file = &self.files[*index];
                (delete_first_rank(&file.path), 1, 0, file.path.clone())
            }
        }
    }

    pub(crate) fn files_in(&self, folder: &str) -> std::ops::Range<usize> {
        let prefix = format!("{folder}/");
        let start = self
            .files
            .partition_point(|file| file.path.as_str() < prefix.as_str());
        let end = start
            + self.files[start..].partition_point(|file| file.path.starts_with(prefix.as_str()));
        start..end
    }

    fn deletable_file(&self, index: usize) -> bool {
        !self.copies[index].is_empty()
    }

    /// The top-most whole folders and loose files that could go, then a
    /// greedy pass: a piece is accepted only while each of its files keeps
    /// an identical copy that is neither in it nor already accepted, so one
    /// copy of everything always stays. A folder that fails is tried again
    /// piece by piece.
    fn select_deletions(&mut self) -> Vec<Unit> {
        let mut candidates = Vec::new();
        self.collect_candidates("", &mut candidates);
        candidates.sort_by_key(|unit| self.unit_key(unit));
        let mut accepted = Vec::new();
        for unit in candidates {
            self.consider(unit, &mut accepted);
        }
        accepted
    }

    fn collect_candidates(&self, folder: &str, candidates: &mut Vec<Unit>) {
        for subfolder in self.subfolders.get(folder).into_iter().flatten() {
            if self.covered(subfolder) {
                candidates.push(Unit::Folder(subfolder.clone()));
            } else {
                self.collect_candidates(subfolder, candidates);
            }
        }
        for &index in self.direct_files.get(folder).into_iter().flatten() {
            if self.deletable_file(index) {
                candidates.push(Unit::File(index));
            }
        }
    }

    fn consider(&mut self, unit: Unit, accepted: &mut Vec<Unit>) {
        let members: Vec<usize> = match &unit {
            Unit::Folder(folder) => self.files_in(folder).collect(),
            Unit::File(index) => vec![*index],
        };
        let inside = |other: usize| match &unit {
            Unit::Folder(folder) => is_inside(&self.files[other].path, folder),
            Unit::File(index) => other == *index,
        };
        let keeps_a_copy = members.iter().all(|&member| {
            self.copies[member]
                .iter()
                .any(|&other| !self.deleted[other] && !inside(other))
        });
        if keeps_a_copy {
            for member in members {
                self.deleted[member] = true;
            }
            accepted.push(unit);
            return;
        }
        if let Unit::Folder(folder) = unit {
            let mut pieces: Vec<Unit> = self
                .subfolders
                .get(&folder)
                .into_iter()
                .flatten()
                .map(|subfolder| Unit::Folder(subfolder.clone()))
                .chain(
                    self.direct_files
                        .get(&folder)
                        .into_iter()
                        .flatten()
                        .filter(|&&index| self.deletable_file(index))
                        .map(|&index| Unit::File(index)),
                )
                .collect();
            pieces.sort_by_key(|piece| self.unit_key(piece));
            for piece in pieces {
                self.consider(piece, accepted);
            }
        }
    }
}

/// How many folders two paths share from the top.
fn common_folder_depth(left: &str, right: &str) -> usize {
    ancestors(left)
        .into_iter()
        .zip(ancestors(right))
        .take_while(|(left_folder, right_folder)| left_folder == right_folder)
        .count()
}
