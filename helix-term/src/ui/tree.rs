use std::cell::Cell;
use std::collections::HashMap;
use std::fmt::{self, Debug};
use std::hash::Hash;
use std::mem::take;

use anyhow::{anyhow, bail};

#[cfg(test)]
mod tests;

pub trait TreeData {
    type Node: Ord + PartialEq + Debug + Clone + Hash + Eq + 'static;
    type NodeIter<'a>: Iterator<Item = (Self::Node, bool)> + 'a
    where
        Self: 'a;
    /// Returns all children of a (non-leave) tree node. This function is only
    /// called when necessary and may perform IO intense operations.
    ///
    /// # Returns
    ///
    /// An iterator that yields all (direct) children of `path` and whether
    /// they are leaves: `(node, is_leave)`. Thes returned items don't need to
    /// be sorted.
    ///
    /// # Example
    ///
    /// If a directory `foo` containgin files `foo1`, `foo2` and the directory
    /// `bar` then the FS `TreeData` implementation yields: `[("foo1", true),
    /// ("foo2", true), ("bar", false)]`
    fn expand(&mut self, path: &[Self::Node]) -> anyhow::Result<Self::NodeIter<'_>>;
}

/// entiel value used for parent idx of root items
const NO_PARENT: usize = usize::MAX;

#[derive(Debug, PartialEq, Clone)]
struct Node<N> {
    path: Box<[N]>,
    children: usize,
    /// lazily maintained cache of this nodes parent
    parent_idx_cache: Cell<usize>,
    expanded: bool,
    show_children: bool,
}

impl<N: Ord> Node<N> {
    fn name(&self) -> &N {
        &self
            .path
            .last()
            .expect("Every path must have atleast one element")
    }

    fn parent_idx(&self, mut items: &[Self]) -> usize {
        let parent_path = &self.path[..self.path.len() - 1];
        // check if we need to refresh the cache because new tree elements were inserted
        if self.parent_idx_cache.get() != NO_PARENT
            && &*items[self.parent_idx_cache.get()].path != parent_path
        {
            let parent_idx = items
                .binary_search_by_key(&parent_path, |item| &item.path)
                .expect("parent must be in tree");
            self.parent_idx_cache.set(parent_idx)
        }
        self.parent_idx_cache.get()
    }
}

#[derive(PartialEq)]
pub struct Tree<T: TreeData> {
    items: Vec<Node<T::Node>>,
    selection: usize,
    data_model: T,
}

impl<T: TreeData> Tree<T> {
    pub fn new(data_model: T) -> Tree<T> {
        let mut tree = Tree {
            items: Vec::with_capacity(1024),
            selection: 0,
            data_model,
        };
        tree.refresh();
        tree
    }
    pub fn collpase(&mut self, idx: usize) {
        self.items[idx].show_children = false
    }

    pub fn expand(&mut self, idx: usize) -> anyhow::Result<()> {
        let item = &mut self.items[idx];
        item.show_children = true;
        if item.expanded {
            return Ok(());
        }
        item.expanded = true;

        let path = item.path.to_vec();
        let old_len = self.items.len();
        let chidren_start = idx + 1;
        let children = self.data_model.expand(&path)?.map(|(child, is_leaf)| Node {
            path: {
                let mut path = path.clone();
                path.push(child);
                path.into_boxed_slice()
            },
            children: 0,
            // leaves can never be expanded
            expanded: is_leaf,
            show_children: false,
            parent_idx_cache: Cell::new(idx),
        });
        self.items.splice(chidren_start..chidren_start, children);
        let num_children = self.items.len() - old_len;
        self.items[chidren_start..chidren_start + num_children]
            .sort_unstable_by(|node1, node2| node1.path.cmp(&node2.path));
        if num_children != 0 {
            for ancestor in self.ancestors_mut(idx) {
                ancestor.children += num_children;
                ancestor.show_children = true;
            }
        }

        Ok(())
    }

    pub fn reveal_path(&mut self, path: &[T::Node]) -> anyhow::Result<usize> {
        for depth in 1..path.len() {
            let path = &path[..depth];
            let Ok(item_index) = self
                .items
                .binary_search_by_key(&path, |item| &item.path)
             else {
                bail!("path not found");
            };
            self.expand(item_index)?;
        }
        self.items
            .binary_search_by_key(&path, |item| &item.path)
            .map_err(|_| anyhow!("not found"))
    }

    pub fn refresh(&mut self) {
        let Ok(root_nodes) = self.data_model.expand(&[]) else {
            self.items = Vec::new();
            self.selection = 0;
            return;
        };
        // we keep a mapping from paths (that are not leaves) to new ids so we
        // can check whether they exist in the new tree and if we should expand
        // them (plus some additional info)
        let mut unexpanded_nodes: HashMap<Box<[T::Node]>, usize> = HashMap::new();
        let mut new_items: Vec<Node<T::Node>> = root_nodes
            .enumerate()
            .map(|(i, (child, is_leaf))| {
                let path = Box::new([child]);
                if !is_leaf {
                    unexpanded_nodes.insert(path.clone(), i);
                }
                Node {
                    path,
                    children: 0,
                    // leaves can never be expanded
                    expanded: is_leaf,
                    show_children: false,
                    parent_idx_cache: Cell::new(NO_PARENT),
                }
            })
            .collect();
        let old_selection = &self.items.get(self.selection);

        let mut i = 0;
        while i < self.items.len() {
            let item = &self.items[i];
            let Some(new_idx) = unexpanded_nodes.remove(&item.path) else{
                // this inner node does not exist anymore so ignore its children
                i += 1 + item.children;
                continue;
            };
            println!("refreshing {i}");
            let new_item = &mut new_items[new_idx];
            new_item.expanded = true;
            let Ok(children) = self.data_model.expand(&item.path) else {
                continue;
            };
            new_item.show_children = item.show_children;
            let old_len = new_items.len();
            new_items.extend(children.enumerate().map(|(i, (child, is_leaf))| {
                let mut path = item.path.to_vec();
                path.push(child);
                let path = path.into_boxed_slice();
                if !is_leaf {
                    unexpanded_nodes.insert(path.clone(), old_len + i);
                }
                Node {
                    path,
                    children: 0,
                    // leaves can never be expanded
                    expanded: is_leaf,
                    show_children: false,
                    parent_idx_cache: Cell::new(new_idx),
                }
            }));
            let num_children = new_items.len() - old_len;
            if num_children != 0 {
                for ancestor in AncestorsMut::new(&mut new_items, new_idx) {
                    ancestor.children += num_children;
                }
            }
            i += 1;
        }
        new_items.sort_unstable_by(|node1, node2| node1.path.cmp(&node2.path));
        self.selection = old_selection
            .and_then(|it| {
                new_items
                    .binary_search_by_key(&&it.path, |node| &node.path)
                    .ok()
            })
            .unwrap_or(0);
        self.items = new_items;
    }

    fn ancestors_mut(&mut self, idx: usize) -> AncestorsMut<'_, T::Node> {
        AncestorsMut::new(&mut self.items, idx)
    }

    fn ancestors(&self, idx: usize) -> Ancestors<'_, T::Node> {
        Ancestors::new(&self.items, idx)
    }

    fn render(&self, mut start: usize, mut height: usize, mut draw_line: impl FnMut(&[T::Node])) {
        if self.items.is_empty() {
            return;
        }

        let max_sticky_ancestor = 10;
        let ancestors: Vec<_> = self
            .ancestors(self.selection)
            .take(max_sticky_ancestor)
            .collect();

        for (ancestor_idx, ancestor) in ancestors.into_iter().rev() {
            if ancestor_idx >= start {
                break;
            }
            if height == 0 {
                return;
            }
            draw_line(&ancestor.path);
            start += 1;
            height -= 1;
        }
        let mut i = start;
        while i < self.items.len() && height > 0 {
            let item = &self.items[i];
            draw_line(&item.path);
            i += 1;
            height -= 1;
            if !item.show_children {
                i += item.children
            }
        }
    }
}

impl<T: TreeData> Debug for Tree<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entries(self.items.iter().map(|node| &node.path))
            .finish()
    }
}

struct AncestorsMut<'a, N> {
    items: &'a mut [Node<N>],
    item: Option<&'a mut Node<N>>,
}
impl<'a, N> AncestorsMut<'a, N> {
    fn new(items: &'a mut [Node<N>], idx: usize) -> Self {
        let (item, items): (&'a mut _, &'a mut _) =
            items[..=idx].split_last_mut().expect("idx is in bounds");
        AncestorsMut {
            items,
            item: Some(item),
        }
    }
}

impl<'a, N: Ord> Iterator for AncestorsMut<'a, N> {
    type Item = &'a mut Node<N>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.item.take()?;
        let next_idx = item.parent_idx(self.items);
        if next_idx != NO_PARENT {
            // using take is necessary to make borrow checker happy
            let (item, items) = take(&mut self.items)[..=next_idx].split_last_mut().unwrap();
            self.items = items;
            self.item = Some(item);
        }
        Some(item)
    }
}

struct Ancestors<'a, N> {
    items: &'a [Node<N>],
    item: Option<(usize, &'a Node<N>)>,
}
impl<'a, N> Ancestors<'a, N> {
    fn new(items: &'a [Node<N>], idx: usize) -> Self {
        Ancestors {
            items: &items[..idx],
            item: Some((idx, &items[idx])),
        }
    }
}

impl<'a, N: Ord> Iterator for Ancestors<'a, N> {
    type Item = (usize, &'a Node<N>);

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.item.take()?;
        let next_idx = item.1.parent_idx(self.items);
        if next_idx != NO_PARENT {
            // using take is necessary to make borrow checker happy
            self.item = Some((next_idx, &self.items[next_idx]));
            self.items = &self.items[..next_idx];
        }
        Some(item)
    }
}
