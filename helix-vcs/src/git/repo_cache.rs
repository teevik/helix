use std::ops::Index;
use std::path::Path;
use std::sync::Arc;

use git_repository::ThreadSafeRepository;

use crate::git::repo_cache::dir_tree::{DirectoryTree, MissingDirTreeEntry};

use super::open_repo;

mod dir_tree;

pub struct RepoCache {
    dir_tree: DirectoryTree<CacheStatus>,
    repos: Vec<Arc<ThreadSafeRepository>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct CacheSlot(u32);

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum CacheStatus {
    /// This directory contains a repository
    HasRepo(CacheSlot),
    /// This directory does not contain a repository
    NoRepo,
    /// This directory was not checked for a repository yet
    Unresolved,
}

impl CacheStatus {
    fn is_resolved(self) -> bool {
        self != CacheStatus::Unresolved
    }
}

struct CacheLookupError<'a> {
    last_matched_cache: Option<(dir_tree::NodeId, &'a Path)>,
    cause: CacheLookupErrorCause<'a>,
}

enum CacheLookupErrorCause<'a> {
    NoCache(dir_tree::NodeId),
    MissingEntry(MissingDirTreeEntry<'a>),
}

impl RepoCache {
    fn open_repo(&mut self, path: &Path, ceiling_dir: Option<&Path>) -> CacheStatus {
        match open_repo(path, ceiling_dir) {
            Some(repo) => {
                let slot = CacheSlot(self.repos.len() as u32);
                CacheStatus::HasRepo(slot)
            }
            None => CacheStatus::NoRepo,
        }
    }

    fn insert_directory(
        &self,
        path: &Path,
        lookup_result: CacheLookupError,
    ) -> Option<Arc<ThreadSafeRepository>> {
        let cache = if let Some((cached_node, path_from_cache)) = lookup_result.last_matched_cache {
            let is_cached = matches!(
                self.dir_tree[cached_node].content,
                CacheStatus::HasRepo(repo) if directory_in_repo(&*self[repo], path_from_cache)
            );

            if is_cached {
                self.dir_tree[cached_node].content
            } else {
                // only look up to the last cached directory to avoid repetetly walking to full fs
                let ceiling_dir = strip_postfix(path, path_from_cache);
                self.open_repo(path, Some(ceiling_dir))
            }
        } else {
            // first time looking up any directory requires full fs walk
            self.open_repo(path, None)
        };

        repo
    }

    fn lookup_directory<'p>(
        &self,
        path: &'p Path,
    ) -> Result<Option<Arc<ThreadSafeRepository>>, CacheLookupError<'p>> {
        let mut last_matched_cache = None;

        let res = self.dir_tree.walk_path(path, |node, remaining_path| {
            if self.dir_tree[node].content.is_resolved() {
                last_matched_cache = Some((node, remaining_path))
            }
        });

        let node = match res {
            Ok(node) => node,
            Err(missing_entry) => {
                return Err(CacheLookupError {
                    last_matched_cache,
                    cause: CacheLookupErrorCause::MissingEntry(missing_entry),
                })
            }
        };

        match self.dir_tree[node].content {
            CacheStatus::NoRepo => Ok(None),
            CacheStatus::HasRepo(slot) => Ok(Some(self[slot].clone())),
            CacheStatus::Unresolved => Err(CacheLookupError {
                last_matched_cache,
                cause: CacheLookupErrorCause::NoCache(node),
            }),
        }
    }
}

impl Index<CacheSlot> for RepoCache {
    type Output = Arc<ThreadSafeRepository>;

    fn index(&self, id: CacheSlot) -> &Self::Output {
        &self.repos[id.0 as usize]
    }
}

/// Returns a path `res` such that, `res.join(postfix)` yields the original `path`.
/// Compared to `Path::sptrix_sprefix` this function assumes assumes that
/// the provided `postfix` actually terminates the `path` (`path.ends_with(postfix)`).
fn strip_postfix<'a>(path: &'a Path, postfix: &Path) -> &'a Path {
    debug_assert!(
        path.ends_with(postfix),
        "{} is not a postfix of {}",
        postfix.display(),
        path.display()
    );
    let mut path = path.components();
    for component in postfix.components().rev() {
        path.next_back();
    }
    path.as_path()
}
