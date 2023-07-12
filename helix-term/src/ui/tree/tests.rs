use std::collections::{hash_map, HashMap};
use std::{iter, slice};

use crate::ui::tree::{Tree, TreeData};

struct TestData(HashMap<Box<[&'static str]>, HashMap<&'static str, bool>>);
impl TestData {
    fn new(paths: &[&'static str]) -> Self {
        let mut paths = paths.to_vec();
        paths.sort_unstable();
        let mut tree = HashMap::new();
        for path in paths {
            let components: Vec<_> = path.split('/').collect();
            for i in 1..=components.len() {
                let chidren: &mut HashMap<_, _> = tree
                    .entry(components[..i - 1].to_vec().into_boxed_slice())
                    .or_default();
                *chidren.entry(components[i - 1]).or_insert(true) &= components.len() == i;
            }
        }
        Self(tree)
    }
}
impl TreeData for TestData {
    type Node = &'static str;

    type NodeIter<'a> = hash_map::IntoIter<&'static str, bool>;

    fn expand(&mut self, path: &[Self::Node]) -> anyhow::Result<Self::NodeIter<'_>> {
        self.0
            .get(path)
            .map(|chidren| chidren.clone().into_iter())
            .ok_or(anyhow::anyhow!("not found"))
    }
}

const TEST_TREE_HEIGHT: usize = 3;
const TEST_TREE_SCROLLOFF: usize = 1;
fn tree(paths: &[&'static str]) -> Tree<TestData> {
    let tree = Tree::new(TestData::new(paths), TEST_TREE_HEIGHT, TEST_TREE_SCROLLOFF);
    tree
}
macro_rules! assert_eq_tree {
    ($left:expr, $right:literal $($tt: tt)*) => {
        let left = format!("{:?}", $left);
        println!("{left}");
        assert_eq!(left, $right $($tt: tt)*);
    };
}

#[test]
fn construct() {
    let tree = tree(&["foo", "foo/bar", "foo/bar/1", "foobar", "foo/bar2"]);
    assert_eq_tree!(tree, r#"{["foo"], ["foobar"]}"#);
}

#[test]
fn expand() {
    let mut tree = tree(&["foo", "foo/bar", "foo/bar/3", "foobar", "foo/bar2"]);
    assert_eq!(format!("{tree:?}"), r#"{["foo"], ["foobar"]}"#);
    tree.expand(0).unwrap();
    assert_eq_tree!(
        tree,
        r#"{["foo"], ["foo", "bar"], ["foo", "bar2"], ["foobar"]}"#
    );
    tree.expand(2).unwrap();
    tree.expand(1).unwrap();
    assert_eq_tree!(
        tree,
        r#"{["foo"], ["foo", "bar"], ["foo", "bar", "3"], ["foo", "bar2"], ["foobar"]}"#
    );
}

#[test]
fn refresh() {
    let mut tree1 = tree(&["foo", "foo/bar", "foo/bar/3", "foobar", "foo/bar2"]);
    tree1.expand(0).unwrap();
    tree1.expand(2).unwrap();
    tree1.expand(1).unwrap();
    assert_eq_tree!(
        tree1,
        r#"{["foo"], ["foo", "bar"], ["foo", "bar", "3"], ["foo", "bar2"], ["foobar"]}"#
    );
    tree1.refresh();
    assert_eq_tree!(
        tree1,
        r#"{["foo"], ["foo", "bar"], ["foo", "bar", "3"], ["foo", "bar2"], ["foobar"]}"#
    );
    let mut tree2 = tree(&["foo", "foo/bar", "foo/bar/3", "foobar", "foo/bar2"]);
    tree2.refresh();
    assert_eq_tree!(tree2, r#"{["foo"], ["foobar"]}"#);
}

#[test]
fn reveal() {
    let mut tree = tree(&["foo", "foo/bar", "foo/bar/3", "foobar", "foo/bar2/test"]);
    let idx = tree.reveal_path(&["foo", "bar", "3"]).unwrap();
    assert_eq_tree!(
        tree,
        r#"{["foo"], ["foo", "bar"], ["foo", "bar", "3"], ["foo", "bar2"], ["foobar"]}"#
    );
    assert_eq!(idx, 2)
}

macro_rules! assert_selection {
    ($tree:expr, $selection:literal, $top:literal) => {
        println!("{:?}", $tree);
        assert_eq!($tree.selection, $selection, "selection");
        assert_eq!($tree.top, $top, "last");
    };
}
#[test]
fn set_selection() {
    let mut tree = tree(&["foo", "foo/bar", "foo/bar/3", "foobar", "foo/bar2/test"]);
    tree.set_selection(0);
    assert_selection!(tree, 0, 0);
    tree.set_selection(1);
    assert_selection!(tree, 1, 0);
    tree.expand(0).unwrap();
    assert_selection!(tree, 3, 1);
    tree.collapse(0);
    assert_selection!(tree, 3, 0);
    tree.move_up();
    assert_selection!(tree, 0, 0);
    println!("x");
    tree.expand(0).unwrap();
    assert_selection!(tree, 0, 0);
    tree.move_down();
    assert_selection!(tree, 1, 0);
    tree.collapse(0);
    assert_selection!(tree, 0, 0);
    tree.move_down();
    assert_selection!(tree, 3, 0);
}
