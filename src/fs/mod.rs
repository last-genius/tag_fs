use fuser::{
    Filesystem, KernelConfig, ReplyAttr, ReplyBmap, ReplyCreate, ReplyData, ReplyDirectory,
    ReplyDirectoryPlus, ReplyEmpty, ReplyEntry, ReplyIoctl, ReplyLock, ReplyLseek, ReplyOpen,
    ReplyStatfs, ReplyWrite, ReplyXattr, Request, TimeOrNow,
};
use libc::{c_int, ENOENT, ENOSYS};
use log::debug;
use sha3::{Digest, Sha3_256};
use std::cmp::min;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{create_dir_all, remove_file, File, OpenOptions};
use std::os::unix::fs::{self, FileExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use uuid::Uuid;

use self::defs::TTL;
use self::nodes::{FileNode, INode, NameNode, Node, TagNode};

mod defs;
mod nodes;

pub struct TagFS {
    hasher: Sha3_256,
    data_dir: PathBuf,
}

impl TagFS {
    pub fn new() -> Self {
        let base_path = PathBuf::from("/tmp/tagfs");
        for subdir in [
            "inodes",
            "namenodes",
            "namenodes_id",
            "filenodes",
            "tagnodes",
        ] {
            create_dir_all(base_path.join(subdir)).unwrap();
        }

        let mut fs = Self {
            hasher: Sha3_256::new(),
            data_dir: base_path,
        };

        // TODO temporary testing
        debug!("starting serialization");

        let mut fake_root = TagNode::new(1);
        let file_node = FileNode::new(&mut fs.hasher, 2);
        let name_node = NameNode::new("file1".into(), Node::File(file_node.hash.clone()));
        fake_root.add_file(&name_node);

        let path = Path::new(&fs.data_dir).join("filenodes").join("test_file");
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        bincode::serialize_into(file, &file_node).unwrap();

        debug!("finished serialization");

        debug!("{file_node}");
        let file = OpenOptions::new().read(true).open(&path).unwrap();
        let deserialized_file: FileNode = bincode::deserialize_from(file).unwrap();

        // Finish of temporary testing

        fs
    }

    fn get_inode(&self, ino: u64) -> Result<INode, c_int> {
        debug!("get_inode | {ino}");
        let path = self.data_dir.join("inodes").join(ino.to_string());
        if let Ok(file) = File::open(&path) {
            Ok(INode::File(bincode::deserialize_from(file).unwrap()))
        } else {
            Err(libc::ENOENT)
        }
    }

    fn get_name_node(&self, id: &Uuid) -> Result<NameNode, c_int> {
        debug!("get_name_node | {id}");
        let path = self.data_dir.join("namenodes_id").join(id.to_string());
        if let Ok(file) = File::open(&path) {
            Ok(bincode::deserialize_from(file).unwrap())
        } else {
            Err(libc::ENOENT)
        }
    }

    fn get_node(&self, link_node: &Node) -> Result<INode, c_int> {
        debug!("get_node | {link_node}");

        match link_node {
            Node::File(hash) => {
                let path = self.data_dir.join("filenodes").join(hash.code.to_string());
                if let Ok(file) = File::open(&path) {
                    Ok(INode::File(bincode::deserialize_from(file).unwrap()))
                } else {
                    Err(libc::ENOENT)
                }
            }
            Node::Tag(id) => {
                let path = self.data_dir.join("tagnodes").join(id.to_string());
                if let Ok(file) = File::open(&path) {
                    Ok(INode::Tag(bincode::deserialize_from(file).unwrap()))
                } else {
                    Err(libc::ENOENT)
                }
            }
        }
    }

    pub fn insert_inode(&mut self, node: &INode) {
        match node {
            INode::Tag(f) => self.write_tag_node(f),
            INode::File(t) => self.write_file_node(t),
        }
    }

    fn write_file_node(&self, inode: &FileNode) {
        debug!("write_file_node | {inode}");

        let path = Path::new(&self.data_dir)
            .join("filenodes")
            .join(inode.hash.code.clone());
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        bincode::serialize_into(file, inode).unwrap();

        let symlink_path = Path::new(&self.data_dir)
            .join("inodes")
            .join(inode.file_attr.inode.to_string());

        remove_file(&symlink_path).unwrap();
        fs::symlink(path, symlink_path).unwrap();
    }

    fn write_tag_node(&self, inode: &TagNode) {
        debug!("write_tag_node | {inode}");

        let path = Path::new(&self.data_dir)
            .join("tagnodes")
            .join(inode.id.to_string());
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        bincode::serialize_into(file, inode).unwrap();

        let symlink_path = Path::new(&self.data_dir)
            .join("inodes")
            .join(inode.dir_attr.inode.to_string());

        remove_file(&symlink_path).unwrap();
        fs::symlink(path, symlink_path).unwrap();
    }

    pub fn insert_name_node(&mut self, name_node: &NameNode) {
        debug!("insert_name_node | {name_node}");

        // BTreeSet by name
        let path = Path::new(&self.data_dir)
            .join("namenodes")
            .join(name_node.name.clone());

        let mut b = BTreeSet::new();

        if path.exists() {
            let file = OpenOptions::new().read(true).open(&path).unwrap();
            b = bincode::deserialize_from(file).unwrap();
        }

        b.insert(name_node.id);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        bincode::serialize_into(file, &b).unwrap();

        // By UUID
        let path = Path::new(&self.data_dir)
            .join("namenodes_id")
            .join(name_node.id.to_string());
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        bincode::serialize_into(file, name_node).unwrap();
    }

    // Service functions

    pub fn search_name(&self, tag_node: &TagNode, os_name: &OsStr) -> Option<INode> {
        for id in &tag_node.dir_links {
            if let Ok(name_node) = self.get_name_node(id) {
                if &name_node.name == os_name {
                    if let Ok(node) = self.get_node(&name_node.link) {
                        return Some(node);
                    } else {
                        continue;
                    }
                }
            }
        }

        None
    }
}

impl Filesystem for TagFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        debug!(
            "lookup | parent: {}; name: {}",
            parent,
            name.to_str().unwrap()
        );
        // TODO: I think the trick here could be to use the parent i-node to denote a temporary
        // file that has all the metadata we need (the content of the current tag requests)
        //let fake_root_dir_attr = InodeAttributes::new_file_attr(1, FileKind::Directory, 0x755);
        let os_name = &name.to_os_string();

        // Iterate through every name node we point to, check whether any names are the same
        // TODO: Instead of just pointing to UUIDs possibly point to names too to speed this up?
        if let Ok(x) = self.get_inode(parent) {
            if let INode::Tag(t) = x {
                if let Some(node) = self.search_name(&t, os_name) {
                    match node {
                        INode::File(f) => {
                            reply.entry(&TTL, &f.file_attr.into(), 0);
                        }
                        INode::Tag(t) => {
                            reply.entry(&TTL, &t.dir_attr.into(), 0);
                        }
                    }
                    return;
                }
            }
        }

        reply.error(ENOENT);
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        debug!("getattr | ino: {}", ino);
        if let Ok(node) = self.get_inode(ino) {
            match node {
                INode::File(f) => reply.attr(&TTL, &f.file_attr.into()),
                INode::Tag(t) => reply.attr(&TTL, &t.dir_attr.into()),
            }
        } else {
            reply.error(ENOENT);
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        debug!("read | ino: {}; offset: {}", ino, offset);
        let mut path = PathBuf::from(&self.data_dir);
        if let Ok(node) = self.get_inode(ino) {
            match node {
                INode::File(f) => {
                    path.push("filenodes");
                    path.push(f.hash.code.clone());
                }
                INode::Tag(t) => {
                    path.push("tagnodes");
                    path.push(t.id.to_string());
                }
            }

            if let Ok(file) = File::open(&path) {
                let file_size = file.metadata().unwrap().len();
                // Could underflow if file length is less than local_start
                let read_size = min(size, file_size.saturating_sub(offset as u64) as u32);

                let mut buffer = vec![0; read_size as usize];
                file.read_exact_at(&mut buffer, offset as u64).unwrap();
                reply.data(&buffer);
            } else {
                reply.error(ENOENT);
            }
        } else {
            reply.error(ENOENT);
        }
    }

    fn readdir(&mut self, _req: &Request, ino: u64, _fh: u64, offset: i64, reply: ReplyDirectory) {
        debug!("readdir | ino: {}; offset: {}", ino, offset);

        //TODO: implement file lookup by hash
        //for (i, entry) in file.back_links.iter().enumerate().skip(offset as usize) {
        //let e = entry.upgrade().unwrap();
        //let x = RefCell::borrow(&e);
        //// i + 1 means the index of the next entry
        //// i-node, offset, type, name
        //if reply.add(ino, (i + 1) as i64, FileType::RegularFile, &x.name) {
        //break;
        //}
        //}

        //Node::Tag(_x) => {
        // TODO
        //let tag = RefCell::borrow(&x);
        //if tag.dir_attr.inode == ino {
        //for (i, entry) in tag.dir_links.iter().enumerate().skip(offset as usize) {
        //let x = RefCell::borrow(entry);
        //// i + 1 means the index of the next entry
        //// i-node, offset, type, name
        //match &x.link {
        //Node::Tag(y) => {
        //if reply.add(
        //RefCell::borrow(&y).dir_attr.inode,
        //(i + 1) as i64,
        //FileType::Directory,
        //&x.name,
        //) {
        //break;
        //}
        //}
        //Node::File(y) => {
        //if reply.add(
        //RefCell::borrow(&y).file_attr.inode,
        //(i + 1) as i64,
        //FileType::RegularFile,
        //&x.name,
        //) {
        //break;
        //}
        //}
        //}
        //}
        //}
        //reply.ok();
        //return;
        //}
        reply.error(ENOENT);
    }

    // NOTE: All the calls below this point are unimplemented, and return their default return
    // values, while also debug printing some information so we could use that while developing and
    // determining which functions need to be implemented for certain functionality to work
    //
    // TODO: Figure out what exactly is needed for simple functionality
    // As far as I can tell:
    //  * file creation (touch calls lookup -> create -> mknod -> lookup)
    //  * file attributes changing
    //  * "directory" creation
    //  * moving files and tags

    fn init(&mut self, _req: &Request<'_>, _config: &mut KernelConfig) -> Result<(), c_int> {
        // TODO: Initiate hashers, lists, etc.
        // TODO: In future, recover data from a disk image?
        debug!("init");

        // Create a fake root dir (sort of like 'all tags')
        let mut fake_root = TagNode::new(1);

        // Create a simple test file too
        let file_node = FileNode::new(&mut self.hasher, 2);
        let name_node = NameNode::new("file1".into(), Node::File(file_node.hash.clone()));
        fake_root.add_file(&name_node);

        self.insert_inode(&INode::File(file_node));
        self.insert_inode(&INode::Tag(fake_root));
        self.insert_name_node(&name_node);

        Ok(())
    }

    fn destroy(&mut self) {
        debug!("destroy");
    }

    fn forget(&mut self, _req: &Request<'_>, _ino: u64, _nlookup: u64) {
        debug!("forget");
    }

    fn setattr(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        _size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        debug!("setattr");
        reply.error(ENOSYS);
    }

    fn readlink(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyData) {
        debug!("readlink");
        reply.error(ENOSYS);
    }

    fn mknod(
        &mut self,
        _req: &Request<'_>,
        _parent: u64,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        debug!("mknod");
        reply.error(ENOSYS);
    }

    fn mkdir(
        &mut self,
        _req: &Request<'_>,
        _parent: u64,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        debug!("mkdir");
        reply.error(ENOSYS);
    }

    fn unlink(&mut self, _req: &Request<'_>, _parent: u64, _name: &OsStr, reply: ReplyEmpty) {
        debug!("unlink");
        reply.error(ENOSYS);
    }

    fn rmdir(&mut self, _req: &Request<'_>, _parent: u64, _name: &OsStr, reply: ReplyEmpty) {
        debug!("rmdir");
        reply.error(ENOSYS);
    }

    fn symlink(
        &mut self,
        _req: &Request<'_>,
        _parent: u64,
        _name: &OsStr,
        _link: &Path,
        reply: ReplyEntry,
    ) {
        debug!("rmdir");
        reply.error(ENOSYS);
    }

    fn rename(
        &mut self,
        _req: &Request<'_>,
        _parent: u64,
        _name: &OsStr,
        _newparent: u64,
        _newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        debug!("rename");
        reply.error(ENOSYS);
    }

    fn link(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _newparent: u64,
        _newname: &OsStr,
        reply: ReplyEntry,
    ) {
        debug!("link");
        reply.error(ENOSYS);
    }

    fn open(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        debug!("open");
        reply.opened(0, 0);
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        _data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        debug!("write");
        reply.error(ENOSYS);
    }

    fn flush(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _lock_owner: u64,
        reply: ReplyEmpty,
    ) {
        debug!("flush");
        reply.error(ENOSYS);
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        debug!("release");
        reply.ok();
    }

    fn fsync(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        debug!("fsync");
        reply.error(ENOSYS);
    }

    fn opendir(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        debug!("opendir");
        reply.opened(0, 0);
    }

    fn readdirplus(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        reply: ReplyDirectoryPlus,
    ) {
        debug!("readdirplus");
        reply.error(ENOSYS);
    }

    fn releasedir(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _flags: i32,
        reply: ReplyEmpty,
    ) {
        debug!("releasedir");
        reply.ok();
    }

    fn fsyncdir(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        debug!("fsyncdir");
        reply.error(ENOSYS);
    }

    fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyStatfs) {
        debug!("statfs");
        reply.statfs(0, 0, 0, 0, 0, 512, 255, 0);
    }

    fn setxattr(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _name: &OsStr,
        _value: &[u8],
        _flags: i32,
        _position: u32,
        reply: ReplyEmpty,
    ) {
        debug!("setxattr");
        reply.error(ENOSYS);
    }

    fn getxattr(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _name: &OsStr,
        _size: u32,
        reply: ReplyXattr,
    ) {
        debug!("getxattr");
        reply.error(ENOSYS);
    }

    fn listxattr(&mut self, _req: &Request<'_>, _ino: u64, _size: u32, reply: ReplyXattr) {
        debug!("listxattr");
        reply.error(ENOSYS);
    }

    fn removexattr(&mut self, _req: &Request<'_>, _ino: u64, _name: &OsStr, reply: ReplyEmpty) {
        debug!("removexattr");
        reply.error(ENOSYS);
    }

    fn access(&mut self, _req: &Request<'_>, _ino: u64, _mask: i32, reply: ReplyEmpty) {
        debug!("access");
        reply.error(ENOSYS);
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        _parent: u64,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        debug!("create");
        reply.error(ENOSYS);
    }

    fn getlk(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _lock_owner: u64,
        _start: u64,
        _end: u64,
        _typ: i32,
        _pid: u32,
        reply: ReplyLock,
    ) {
        debug!("getlk");
        reply.error(ENOSYS);
    }

    fn setlk(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _lock_owner: u64,
        _start: u64,
        _end: u64,
        _typ: i32,
        _pid: u32,
        _sleep: bool,
        reply: ReplyEmpty,
    ) {
        debug!("setlk");
        reply.error(ENOSYS);
    }

    fn bmap(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _blocksize: u32,
        _idx: u64,
        reply: ReplyBmap,
    ) {
        debug!("bmap");
        reply.error(ENOSYS);
    }

    fn ioctl(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _flags: u32,
        _cmd: u32,
        _in_data: &[u8],
        _out_size: u32,
        reply: ReplyIoctl,
    ) {
        debug!("ioctl");
        reply.error(ENOSYS);
    }

    fn fallocate(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        _length: i64,
        _mode: i32,
        reply: ReplyEmpty,
    ) {
        debug!("fallocate");
        reply.error(ENOSYS);
    }

    fn lseek(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        _whence: i32,
        reply: ReplyLseek,
    ) {
        debug!("lseek");
        reply.error(ENOSYS);
    }

    fn copy_file_range(
        &mut self,
        _req: &Request<'_>,
        _ino_in: u64,
        _fh_in: u64,
        _offset_in: i64,
        _ino_out: u64,
        _fh_out: u64,
        _offset_out: i64,
        _len: u64,
        _flags: u32,
        reply: ReplyWrite,
    ) {
        debug!("copy_file_range");
        reply.error(ENOSYS);
    }
}
