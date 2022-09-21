use std::ffi::{OsStr, OsString};
use std::ops::Index;
use std::path::Path;

pub struct DirectoryTree<C> {
    nodes: Vec<DirTreeNode<C>>,
}

impl<C> Index<NodeId> for DirectoryTree<C> {
    type Output = DirTreeNode<C>;

    fn index(&self, index: NodeId) -> &Self::Output {
        &self.nodes[index.0 as usize]
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct NodeId(u32);

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct NodeChildSlot(u32);

pub struct DirTreeNode<C> {
    pub content: C,
    children: Vec<DirTreeChild>,
}

impl<C> DirTreeNode<C> {
    fn new(content: C) -> DirTreeNode<C> {
        DirTreeNode {
            content,
            children: Vec::new(),
        }
    }
}

impl<C> DirTreeNode<C> {
    fn find_child_dir(&self, name: &OsStr) -> Result<NodeId, NodeChildSlot> {
        self.children
            .binary_search_by_key(&name, |child| &child.dir_name)
            .map(|pos| self.children[pos].node)
            .map_err(|pos| NodeChildSlot(pos as u32))
    }
}

struct DirTreeChild {
    // TODO: intern for better performance
    dir_name: OsString,
    node: NodeId,
}

pub struct MissingDirTreeEntry<'a> {
    node: NodeId,
    missed_child: (&'a OsStr, NodeChildSlot),
    remaining_path: &'a Path,
}

impl<C: Clone> DirectoryTree<C> {
    pub fn walk_path<'p>(
        &self,
        path: &'p Path,
        mut visit_component: impl FnMut(NodeId, &'p Path),
    ) -> Result<NodeId, MissingDirTreeEntry<'p>> {
        debug_assert!(!path.exists() || path.is_dir());
        debug_assert!(path.is_absolute());

        let mut cursor = NodeId(0); // tree root
        let mut path_components = path.components();
        while let Some(component) = path_components.next() {
            let component = component.as_os_str();
            match self[cursor].find_child_dir(component) {
                Ok(node) => {
                    cursor = node;
                    visit_component(cursor, path_components.as_path())
                }
                Err(slot) => {
                    let missing_entry = MissingDirTreeEntry {
                        node: cursor,
                        missed_child: (component, slot),
                        remaining_path: path_components.as_path(),
                    };
                    return Err(missing_entry);
                }
            }
        }

        Ok(cursor)
    }

    fn insert_dir(&mut self, missing_entry: MissingDirTreeEntry, content: C) {
        let (child_name, child_pos) = missing_entry.missed_child;
        self[missing_entry.node].children.insert(
            child_pos.0 as usize,
            DirTreeChild {
                dir_name: child_name.to_owned(),
                node: self.next_node_id(),
            },
        );
        self.nodes.push(DirTreeNode::new(content.clone()));

        let mut path_components = missing_entry.remaining_path.components();

        for component in path_components {
            // add this component as a child to the previous path component
            self.nodes.last_mut().unwrap().children.push(DirTreeChild {
                dir_name: component.as_os_str().to_owned(),
                node: self.next_node_id(),
            });

            self.nodes.push(DirTreeNode::new(content.clone()));
        }
    }

    fn next_node_id(&self) -> NodeId {
        NodeId(self.nodes.len() as u32)
    }
}
