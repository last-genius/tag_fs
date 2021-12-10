use std::{cell::RefCell, cmp::Ordering, ffi::OsString, rc::Rc};

use fuser::{FileAttr, FileType};
use sha3::{
    digest::{generic_array::GenericArray, FixedOutput},
    Digest, Sha3_256,
};

use super::NewFileAttr;

pub type Hash256 = GenericArray<u8, <Sha3_256 as FixedOutput>::OutputSize>;

#[derive(PartialEq, Eq)]
pub struct FileNode {
    // TODO
    //content:
    //metadata schema
    //block references
    //hash per block
    pub file_attr: FileAttr,
    pub hash: Hash256,
}

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
    pub fn new(hasher: &mut Sha3_256, ino: u64) -> Rc<RefCell<FileNode>> {
        let f = Self {
            file_attr: FileAttr::new_file_attr(ino, FileType::RegularFile, 0o644),
            hash: hasher.finalize_reset(),
        };

        Rc::new(RefCell::new(f))
    }

    #[allow(dead_code)]
    pub fn calculate_hashes(&mut self, hasher: &mut Sha3_256) {
        // TODO: Calculate hash of the block of file
        hasher.update(b"abc");

        self.hash = hasher.finalize_reset();
    }
}

#[derive(PartialEq, Eq)]
pub struct TagNode {
    // TODO: tag core etc
    // TODO: links to files
    pub dir_attr: FileAttr,
    pub hash: Hash256,
}

impl Ord for TagNode {
    fn cmp(&self, other: &Self) -> Ordering {
        self.hash.cmp(&other.hash)
    }
}
impl PartialOrd for TagNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl TagNode {
    pub fn new(hasher: &mut Sha3_256, ino: u64) -> Rc<RefCell<TagNode>> {
        let f = Self {
            dir_attr: FileAttr::new_file_attr(ino, FileType::Directory, 0o644),
            hash: hasher.finalize_reset(),
        };

        Rc::new(RefCell::new(f))
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub enum Node {
    File(Rc<RefCell<FileNode>>),
    Tag(Rc<RefCell<TagNode>>),
}

pub struct NameNode {
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
        Self { name, link }
    }
}

// TODO: Merkle-like hash calculation? Therefore instead of a simple list of blocks more elaborate
// structures. Git-like blo[b|ck] operation???

// TODO: Figure out metadata schema stuff

// TODO: FileNameNode
// TODO: TagNode
// TODO: TagNameNode
