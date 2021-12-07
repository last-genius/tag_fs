#![allow(dead_code)]
use sha3::{
    digest::{generic_array::GenericArray, FixedOutput},
    Digest, Sha3_256,
};

// TODO
pub struct HashNode {
    //content:
    //metadata schema
    //block references
    //hash per block
    hash: Option<GenericArray<u8, <Sha3_256 as FixedOutput>::OutputSize>>,
}

impl HashNode {
    pub fn new() -> Self {
        HashNode { hash: None }
    }

    pub fn calculate_hashes(&mut self, hasher: &mut Sha3_256) {
        // TODO: Calculate hash of the block of file
        hasher.update(b"abc");

        self.hash = Some(hasher.finalize_reset());
    }
}

// TODO: Merkle-like hash calculation? Therefore instead of a simple list of blocks more elaborate
// structures. Git-like blo[b|ck] operation???

// TODO: Figure out metadata schema stuff

// TODO: FileNameNode
// TODO: TagNode
// TODO: TagNameNode
