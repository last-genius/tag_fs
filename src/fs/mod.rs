use fuser::{
    FileAttr, FileType, Filesystem, KernelConfig, ReplyAttr, ReplyBmap, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyDirectoryPlus, ReplyEmpty, ReplyEntry, ReplyIoctl, ReplyLock, ReplyLseek,
    ReplyOpen, ReplyStatfs, ReplyWrite, ReplyXattr, Request, TimeOrNow,
};
use libc::{c_int, getgid, getuid, ENOENT, ENOSYS};
use log::debug;
use sha3::{Digest, Sha3_256};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use self::nodes::{FileNode, NameNode, Node, TagNode};

mod nodes;

const TTL: Duration = Duration::from_secs(1);

const HELLO_TXT_CONTENT: &str = "Hello World!\n";

trait NewFileAttr {
    fn new_file_attr(ino: u64, kind: FileType, perm: u16) -> FileAttr;
}

impl NewFileAttr for FileAttr {
    fn new_file_attr(ino: u64, kind: FileType, perm: u16) -> FileAttr {
        FileAttr {
            ino,
            size: 0,
            blocks: 0,
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind,
            perm,
            nlink: 2,
            uid: unsafe { getuid() },
            gid: unsafe { getgid() },
            rdev: 0,
            flags: 0,
            blksize: 512,
        }
    }
}

lazy_static::lazy_static! {
    static ref FAKE_ROOT_DIR_ATTR: FileAttr = FileAttr::new_file_attr(1, FileType::Directory, 0x755);

    static ref HELLO_TXT_ATTR: FileAttr = FileAttr {
        ino: 2,
        size: 13,
        blocks: 1,
        atime: UNIX_EPOCH, // 1970-01-01 00:00:00
        mtime: UNIX_EPOCH,
        ctime: UNIX_EPOCH,
        crtime: UNIX_EPOCH,
        kind: FileType::RegularFile,
        perm: 0o644,
        nlink: 1,
        uid: unsafe { getuid() },
        gid: unsafe { getgid() },
        rdev: 0,
        flags: 0,
        blksize: 512,
    };
}

pub struct TagFS {
    hasher: Sha3_256,
    name_nodes: BTreeMap<OsString, BTreeSet<Rc<RefCell<NameNode>>>>,
    file_nodes: BTreeSet<Node>,
}

impl TagFS {
    pub fn new() -> Self {
        let mut fs = Self {
            hasher: Sha3_256::new(),
            name_nodes: BTreeMap::new(),
            file_nodes: BTreeSet::new(),
        };

        // Create a fake root dir (sort of like 'all tags')
        let fake_root = TagNode::new(1);
        fs.file_nodes.insert(Node::Tag(fake_root.clone()));

        // Create a simple test file too
        let file_node: Rc<RefCell<FileNode>> = FileNode::new(&mut fs.hasher, 2);
        fs.file_nodes.insert(Node::File(file_node.clone()));

        let name_node = NameNode::new("file1".into(), Node::File(file_node));
        fake_root.borrow_mut().add_file(name_node.clone());

        fs.insert_name_node(name_node);

        fs
    }

    // TODO: Figure out proper references. But for now we can just clone this shit
    pub fn insert_name_node(&mut self, name_node: Rc<RefCell<NameNode>>) {
        let name = RefCell::borrow(&name_node).name.clone();
        self.name_nodes
            .entry(name)
            .or_insert(BTreeSet::new())
            .insert(name_node);
    }
}

impl Filesystem for TagFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        debug!(
            "lookup | parent: {}; name: {}",
            parent,
            name.to_str().unwrap()
        );
        let os_name = &name.to_os_string();
        if self.name_nodes.contains_key(os_name) {
            let entry = self.name_nodes[os_name].first();
            if let Some(x) = entry {
                match &RefCell::borrow(&x).link {
                    Node::File(y) => reply.entry(&TTL, &RefCell::borrow(&y).file_attr, 0),
                    Node::Tag(y) => reply.entry(&TTL, &RefCell::borrow(&y).dir_attr, 0),
                }
            }
        } else {
            reply.error(ENOENT);
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        debug!("getattr | ino: {}", ino);
        // TODO: maintain a separate storage for quick inode search????
        for file in &self.file_nodes {
            let file_attr = match file {
                Node::File(x) => RefCell::borrow(&x).file_attr,
                Node::Tag(x) => RefCell::borrow(&x).dir_attr,
            };
            if file_attr.ino == ino {
                reply.attr(&TTL, &file_attr);
                return;
            }
        }
        reply.error(ENOENT);
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        _size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        debug!("read | ino: {}; offset: {}", ino, offset);
        if ino == 2 {
            reply.data(&HELLO_TXT_CONTENT.as_bytes()[offset as usize..]);
        } else {
            reply.error(ENOENT);
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        debug!("readdir | ino: {}; offset: {}", ino, offset);

        for node in &self.file_nodes {
            match node {
                Node::File(x) => {
                    let file = RefCell::borrow(&x);
                    if file.file_attr.ino == ino {
                        // TODO
                        for (i, entry) in file.back_links.iter().enumerate().skip(offset as usize) {
                            let e = entry.upgrade().unwrap();
                            let x = RefCell::borrow(&e);
                            // i + 1 means the index of the next entry
                            // i-node, offset, type, name
                            if reply.add(ino, (i + 1) as i64, FileType::RegularFile, &x.name) {
                                break;
                            }
                        }
                    }
                }
                Node::Tag(x) => {
                    let tag = RefCell::borrow(&x);
                    if tag.dir_attr.ino == ino {
                        // TODO
                        for (i, entry) in tag.dir_links.iter().enumerate().skip(offset as usize) {
                            let x = RefCell::borrow(entry);
                            // i + 1 means the index of the next entry
                            // i-node, offset, type, name
                            match &x.link {
                                Node::Tag(y) => {
                                    if reply.add(
                                        RefCell::borrow(&y).dir_attr.ino,
                                        (i + 1) as i64,
                                        FileType::Directory,
                                        &x.name,
                                    ) {
                                        break;
                                    }
                                }
                                Node::File(y) => {
                                    if reply.add(
                                        RefCell::borrow(&y).file_attr.ino,
                                        (i + 1) as i64,
                                        FileType::RegularFile,
                                        &x.name,
                                    ) {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    reply.ok();
                    return;
                }
            };
        }
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
