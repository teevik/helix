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
    /// they are leaves: `(node, is_leave)`. Thes returned nodes don't need to
    /// be sorted.
    ///
    /// # Example
    ///
    /// If a directory `foo` containgin files `foo1`, `foo2` and the directory
    /// `bar` then the FS `TreeData` implementation yields: `[("foo1", true),
    /// ("foo2", true), ("bar", false)]`
    fn expand(&mut self, path: &[Self::Node]) -> anyhow::Result<Self::NodeIter<'_>>;
}

/// sentiel value used for parent idx of root nodes
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
        self.path
            .last()
            .expect("Every path must have atleast one element")
    }

    fn parent_idx(&self, nodes: &[Self]) -> usize {
        let parent_path = &self.path[..self.path.len() - 1];
        // check if we need to refresh the cache because new tree elements were inserted
        if self.parent_idx_cache.get() != NO_PARENT
            && &*nodes[self.parent_idx_cache.get()].path != parent_path
        {
            let parent_idx = nodes
                .binary_search_by_key(&parent_path, |item| &item.path)
                .expect("parent must be in tree");
            self.parent_idx_cache.set(parent_idx)
        }
        self.parent_idx_cache.get()
    }
}

#[derive(PartialEq)]
pub struct Tree<T: TreeData> {
    nodes: Vec<Node<T::Node>>,
    selection: usize,
    data_model: T,
    scrolloff: usize,
    top: usize,
    height: usize,
}

impl<T: TreeData> Tree<T> {
    pub fn new(data_model: T, height: usize, scrolloff: usize) -> Tree<T> {
        let mut tree = Tree {
            nodes: Vec::with_capacity(1024),
            selection: 0,
            top: 0,
            height,
            scrolloff,
            data_model,
        };
        tree.refresh();
        // necessary to set last
        tree.ensure_selection_visible();
        tree
    }

    pub fn collapse(&mut self, idx: usize) {
        let children = self.nodes[idx].children;
        let depth = self.nodes[idx].path.len();
        for ancestor in self.ancestors_mut(idx + children) {
            if ancestor.path.len() < depth {
                break;
            }
            ancestor.show_children = false
        }
        if self.selection > idx && self.selection <= idx + children {
            self.selection = idx
        }
        if self.top > idx && self.top <= idx + children {
            self.top = idx
        }
        self.ensure_selection_visible();
    }

    pub fn expand(&mut self, idx: usize) -> anyhow::Result<()> {
        let item = &mut self.nodes[idx];
        item.show_children = true;
        if item.expanded {
            self.ensure_selection_visible();
            return Ok(());
        }
        item.expanded = true;

        let path = item.path.to_vec();
        let old_len = self.nodes.len();
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
        self.nodes.splice(chidren_start..chidren_start, children);
        let num_children = self.nodes.len() - old_len;
        self.nodes[chidren_start..chidren_start + num_children]
            .sort_unstable_by(|node1, node2| node1.path.cmp(&node2.path));
        if num_children != 0 {
            for ancestor in self.ancestors_mut(idx) {
                ancestor.children += num_children;
                ancestor.show_children = true;
            }
            if self.top > idx {
                self.top = self.nth(self.top, num_children)
            }
            if self.selection > idx {
                self.selection = self.nth(self.selection, num_children);
            }
            self.ensure_selection_visible();
        }

        Ok(())
    }

    pub fn reveal_path(&mut self, path: &[T::Node]) -> anyhow::Result<usize> {
        for depth in 1..path.len() {
            let path = &path[..depth];
            let Ok(item_index) = self
                .nodes
                .binary_search_by_key(&path, |item| &item.path)
             else {
                bail!("path not found");
            };
            self.expand(item_index)?;
        }
        self.nodes
            .binary_search_by_key(&path, |item| &item.path)
            .map_err(|_| anyhow!("not found"))
    }

    pub fn refresh(&mut self) {
        let Ok(root_nodes) = self.data_model.expand(&[]) else {
            self.nodes = Vec::new();
            self.selection = 0;
            return;
        };
        // we keep a mapping from paths (that are not leaves) to new ids so we
        // can check whether they exist in the new tree and if we should expand
        // them (plus some additional info)
        let mut unexpanded_nodes: HashMap<Box<[T::Node]>, usize> = HashMap::new();
        let mut new_nodes: Vec<Node<T::Node>> = root_nodes
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
        let old_selection = &self.nodes.get(self.selection);

        let mut i = 0;
        while i < self.nodes.len() {
            let item = &self.nodes[i];
            let Some(new_idx) = unexpanded_nodes.remove(&item.path) else{
                // this inner node does not exist anymore so ignore its children
                i += 1 + item.children;
                continue;
            };
            if !item.expanded {
                continue;
            }
            let new_item = &mut new_nodes[new_idx];
            new_item.expanded = true;
            let Ok(children) = self.data_model.expand(&item.path) else {
                continue;
            };
            new_item.show_children = item.show_children;
            let old_len = new_nodes.len();
            new_nodes.extend(children.enumerate().map(|(i, (child, is_leaf))| {
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
            let num_children = new_nodes.len() - old_len;
            if num_children != 0 {
                for ancestor in AncestorsMut::new(&mut new_nodes, new_idx) {
                    ancestor.children += num_children;
                }
            }
            i += 1;
        }
        new_nodes.sort_unstable_by(|node1, node2| node1.path.cmp(&node2.path));
        self.selection = old_selection
            .and_then(|it| {
                new_nodes
                    .binary_search_by_key(&&it.path, |node| &node.path)
                    .ok()
            })
            .unwrap_or(0);
        self.nodes = new_nodes;
    }

    fn ancestors_mut(&mut self, idx: usize) -> AncestorsMut<'_, T::Node> {
        AncestorsMut::new(&mut self.nodes, idx)
    }

    fn ancestors(&self, idx: usize) -> Ancestors<'_, T::Node> {
        Ancestors::new(&self.nodes, idx)
    }

    pub fn set_height(&mut self, height: usize) {
        self.height = height;
        self.ensure_selection_visible();
    }

    pub fn selection(&self) -> Option<usize> {
        (self.selection < self.nodes.len()).then_some(self.selection)
    }

    pub fn top(&self) -> Option<usize> {
        (self.top < self.nodes.len()).then_some(self.top)
    }

    fn ensure_selection_visible(&mut self) {
        if self.nodes.is_empty() || self.height == 0 {
            self.selection = 0;
            self.top = 0;
        }
        let scrolloff = self
            .scrolloff
            .min(self.height.saturating_sub(1) as usize / 2);
        let scrolloff_top = self.nth_rev(self.selection, scrolloff);
        if scrolloff_top < self.top {
            self.top = scrolloff_top
        } else {
            let scrolloff_bot = self.nth_rev(self.selection, self.height - scrolloff);
            if self.top < scrolloff_bot {
                self.top = scrolloff_bot
            }
        }
    }

    pub fn set_selection(&mut self, idx: usize) {
        assert!(idx <= self.nodes.len());
        self.selection = idx;
        self.ensure_selection_visible();
    }

    pub fn move_up(&mut self) {
        if let Some(idx) = self.prev(self.selection) {
            self.set_selection(idx)
        }
    }
    pub fn move_down(&mut self) {
        if let Some(idx) = self.next(self.selection) {
            self.set_selection(idx)
        }
    }

    pub fn nth_rev(&self, mut idx: usize, off: usize) -> usize {
        for _ in 0..off {
            let Some(idx_) = self.prev(idx) else {
               break;
            };
            idx = idx_;
        }
        idx
    }

    pub fn nth(&self, mut idx: usize, off: usize) -> usize {
        for _ in 0..off {
            let Some(idx_) = self.next(idx) else {
               break;
            };
            idx = idx_;
        }
        idx
    }

    pub fn distance(&self, mut start_idx: usize, end_idx: usize) -> Option<usize> {
        assert!(start_idx <= end_idx);
        let mut len = 0;
        while start_idx != end_idx {
            start_idx = self.next(start_idx)?;
            len += 1;
        }
        Some(len)
    }

    pub fn next(&self, idx: usize) -> Option<usize> {
        let mut next = idx + 1;
        if next >= self.nodes.len() {
            return None;
        }
        let item = &self.nodes[idx];
        if !item.show_children {
            next += item.children;
        }
        (next < self.nodes.len()).then_some(next)
    }

    pub fn prev(&self, mut idx: usize) -> Option<usize> {
        if idx == 0 {
            return None;
        }
        idx -= 1;
        Some(self.first_visible_ancestor(idx))
    }

    fn first_visible_ancestor(&self, idx: usize) -> usize {
        self.ancestors(idx)
            .take_while(|(_, ancestor)| !ancestor.show_children)
            .map(|(idx, _)| idx)
            .last()
            .unwrap_or(idx)
    }

    fn render(&self, mut draw_line: impl FnMut(&[T::Node], bool)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut start = self.top;
        let mut height = self.height;

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
            draw_line(&ancestor.path, true);
            start = self.next(start).unwrap_or(self.nodes.len());
            height -= 1;
        }
        let mut i = start;
        while i < self.nodes.len() && height > 0 {
            let item = &self.nodes[i];
            draw_line(&item.path, i == self.selection);
            height -= 1;
            i = self.next(i).unwrap_or(self.nodes.len())
        }
    }
}

impl<T: TreeData> Debug for Tree<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entries(self.nodes.iter().map(|node| &node.path))
            .finish()
    }
}

struct AncestorsMut<'a, N> {
    nodes: &'a mut [Node<N>],
    node: Option<&'a mut Node<N>>,
}
impl<'a, N> AncestorsMut<'a, N> {
    fn new(nodes: &'a mut [Node<N>], idx: usize) -> Self {
        let (node, nodes): (&'a mut _, &'a mut _) =
            nodes[..=idx].split_last_mut().expect("idx is in bounds");
        AncestorsMut {
            nodes,
            node: Some(node),
        }
    }
}

impl<'a, N: Ord> Iterator for AncestorsMut<'a, N> {
    type Item = &'a mut Node<N>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.node.take()?;
        let next_idx = item.parent_idx(self.nodes);
        if next_idx != NO_PARENT {
            // using take is necessary to make borrow checker happy
            let (node, nodes) = take(&mut self.nodes)[..=next_idx].split_last_mut().unwrap();
            self.nodes = nodes;
            self.node = Some(node);
        }
        Some(item)
    }
}

struct Ancestors<'a, N> {
    nodes: &'a [Node<N>],
    node: Option<(usize, &'a Node<N>)>,
}
impl<'a, N> Ancestors<'a, N> {
    fn new(nodes: &'a [Node<N>], idx: usize) -> Self {
        Ancestors {
            nodes: &nodes[..idx],
            node: Some((idx, &nodes[idx])),
        }
    }
}

impl<'a, N: Ord> Iterator for Ancestors<'a, N> {
    type Item = (usize, &'a Node<N>);

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.node.take()?;
        let next_idx = node.1.parent_idx(self.nodes);
        if next_idx != NO_PARENT {
            // using take is necessary to make borrow checker happy
            self.node = Some((next_idx, &self.nodes[next_idx]));
            self.nodes = &self.nodes[..next_idx];
        }
        Some(node)
    }
}
