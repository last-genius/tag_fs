use std::{cmp::Ordering, collections::BTreeSet, ffi::OsString, fmt::Display};

use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_256};
use uuid::Uuid;

use super::defs::{FileKind, Hash256, HashCalculate, InodeAttributes};

#[derive(Serialize, Deserialize)]
pub struct FileNode {
    pub hash: Hash256,
    pub file_attr: InodeAttributes,
    pub back_links: Vec<NameNode>,
}

impl PartialEq for FileNode {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}
impl Eq for FileNode {}
impl Ord for FileNode {
    fn cmp(&self, other: &Self) -> Ordering {
        self.hash.cmp(&other.hash)
    }
}
impl PartialOrd for FileNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl FileNode {
    pub fn new(hasher: &mut Sha3_256, ino: u64, attr: Option<InodeAttributes>) -> Self {
        Self {
            hash: hasher.calculate_hash(),
            file_attr: match attr {
                Some(mut x) => {
                    x.inode = ino;
                    x
                }
                None => InodeAttributes::new_file_attr(ino, FileKind::File, 0o644),
            },
            back_links: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn calculate_hashes(&mut self, hasher: &mut Sha3_256) {
        // TODO: Calculate hash of the block of file
        hasher.update(b"abc");

        self.hash = hasher.calculate_hash();
    }
}

impl Display for FileNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FileNode: {}", self.hash)
    }
}

#[derive(Serialize, Deserialize)]
pub struct TagNode {
    // TODO: links to files
    pub id: Uuid,
    pub dir_attr: InodeAttributes,
    pub back_links: Vec<Uuid>,
    pub dir_links: BTreeSet<Uuid>,
}

impl PartialEq for TagNode {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for TagNode {}
impl Ord for TagNode {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}
impl PartialOrd for TagNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl TagNode {
    pub fn new(ino: u64, attr: Option<InodeAttributes>) -> Self {
        Self {
            id: Uuid::new_v4(),
            dir_attr: match attr {
                Some(mut x) => {
                    x.inode = ino;
                    x
                }
                None => InodeAttributes::new_file_attr(ino, FileKind::Directory, 0o644),
            },
            back_links: Vec::new(),
            dir_links: BTreeSet::new(),
        }
    }

    pub fn add_file(&mut self, name_node: &NameNode) {
        self.dir_links.insert(name_node.id);
    }
}

impl Display for TagNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TagNode: {}", self.id)
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum INode {
    File(FileNode),
    Tag(TagNode),
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Serialize, Deserialize)]
pub enum Node {
    File(Hash256),
    Tag(Uuid),
}

impl INode {
    pub fn to_node(&self) -> Node {
        match self {
            INode::File(f) => Node::File(f.hash.clone()),
            INode::Tag(t) => Node::Tag(t.id),
        }
    }
}

impl Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Node::File(hash) => write!(f, "Node::File({:?})", hash.code),
            Node::Tag(id) => write!(f, "Node::Tag({})", id),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NameNode {
    pub id: Uuid,
    pub name: OsString,
    pub link: Node,
}

impl Ord for NameNode {
    fn cmp(&self, other: &Self) -> Ordering {
        self.name.cmp(&other.name)
    }
}
impl PartialOrd for NameNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for NameNode {
    fn eq(&self, other: &Self) -> bool {
        self.link == other.link
    }
}
impl Eq for NameNode {}

impl NameNode {
    pub fn new(name: OsString, link: Node) -> Self {
        let n = Self {
            id: Uuid::new_v4(),
            name,
            link,
        };

        // TODO
        //match link {
        //Node::File(x) => x.borrow_mut().back_links.push(Rc::downgrade(&name_node)),
        //Node::Tag(x) => x.borrow_mut().back_links.push(Rc::downgrade(&name_node)),
        //};

        n
    }
}

impl Display for NameNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NameNode {:?} -> {}", self.name, self.link)
    }
}

// TODO: Merkle-like hash calculation? Therefore instead of a simple list of blocks more elaborate
// structures. Git-like blo[b|ck] operation???

// TODO: Figure out metadata schema stuff
