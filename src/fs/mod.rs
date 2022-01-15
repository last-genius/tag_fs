use fuser::{
    Filesystem, KernelConfig, ReplyAttr, ReplyBmap, ReplyCreate, ReplyData, ReplyDirectory,
    ReplyDirectoryPlus, ReplyEmpty, ReplyEntry, ReplyIoctl, ReplyLock, ReplyLseek, ReplyOpen,
    ReplyStatfs, ReplyWrite, ReplyXattr, Request, TimeOrNow,
};
use libc::{c_int, EISDIR, ENOENT, ENOSYS};
use log::debug;
use sha3::{Digest, Sha3_256};
use std::cmp::min;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{create_dir_all, File, OpenOptions};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

use crate::fs::defs::{rewrite_symlink, InodeAttributes};

use self::defs::{time_now, FileKind, TTL};
use self::nodes::{FileNode, INode, NameNode, Node, TagNode};

mod defs;
mod nodes;

pub struct TagFS {
    hasher: Sha3_256,
    data_dir: PathBuf,
    inode_cur: u64,
    filehandle_cur: u64,
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

        let fs = Self {
            hasher: Sha3_256::new(),
            data_dir: base_path,
            inode_cur: 1,
            filehandle_cur: 1,
        };

        fs
    }

    fn get_inode_cur(inode_cur: &mut u64) -> u64 {
        let a = *inode_cur;
        *inode_cur += 1;
        a
    }

    fn get_filehandle_cur(&mut self) -> u64 {
        let a = self.filehandle_cur;
        self.filehandle_cur += 1;
        a
    }

    fn allocate_next_inode(
        &mut self,
        inode_kind: FileKind,
        attr: Option<InodeAttributes>,
    ) -> INode {
        debug!("\tallocate_next_inode | {inode_kind:?}");

        match inode_kind {
            FileKind::File => INode::File(FileNode::new(
                &mut self.hasher,
                TagFS::get_inode_cur(&mut self.inode_cur),
                attr,
            )),
            FileKind::Directory => INode::Tag(TagNode::new(
                TagFS::get_inode_cur(&mut self.inode_cur),
                attr,
            )),
            FileKind::Symlink => unimplemented!(),
        }
    }

    fn get_inode(&self, ino: u64) -> Result<INode, c_int> {
        debug!("\tget_inode | {ino}");

        if let Ok(path) = self
            .data_dir
            .join("inodes")
            .join(ino.to_string())
            .read_link()
        {
            if let Ok(file) = File::open(&path) {
                let parent = path.parent().unwrap();
                if parent.ends_with("tagnodes") {
                    return Ok(INode::Tag(bincode::deserialize_from(file).unwrap()));
                } else if parent.ends_with("filenodes") {
                    return Ok(INode::File(bincode::deserialize_from(file).unwrap()));
                }
            }
        }

        Err(libc::ENOENT)
    }

    fn get_node_from_inode(&self, ino: u64) -> Result<Node, c_int> {
        match self.get_inode(ino) {
            Ok(INode::File(f)) => Ok(Node::File(f.hash)),
            Ok(INode::Tag(t)) => Ok(Node::Tag(t.id)),
            Err(_) => Err(libc::ENOENT),
        }
    }

    fn get_name_node(&self, id: &Uuid) -> Result<NameNode, c_int> {
        debug!("\tget_name_node | {id}");
        let path = self.data_dir.join("namenodes_id").join(id.to_string());
        if let Ok(file) = File::open(&path) {
            Ok(bincode::deserialize_from(file).unwrap())
        } else {
            Err(libc::ENOENT)
        }
    }

    fn get_node(&self, link_node: &Node) -> Result<INode, c_int> {
        debug!("\tget_node | {link_node}");

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
        debug!("\twrite_file_node | {inode}");

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

        rewrite_symlink(path, symlink_path);
    }

    fn write_tag_node(&self, inode: &TagNode) {
        debug!("\twrite_tag_node | {inode}");

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

        rewrite_symlink(path, symlink_path);
    }

    pub fn insert_name_node(&mut self, name_node: &NameNode) {
        debug!("\tinsert_name_node | {name_node}");

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
    fn init(&mut self, _req: &Request<'_>, _config: &mut KernelConfig) -> Result<(), c_int> {
        // TODO: Initiate hashers, lists, etc.
        // TODO: In future, recover data from a disk image?
        debug!("init");

        // Create a fake root dir (sort of like 'all tags')
        let mut fake_root = TagNode::new(TagFS::get_inode_cur(&mut self.inode_cur), None);

        // Create a simple test file too
        let file_node = FileNode::new(
            &mut self.hasher,
            TagFS::get_inode_cur(&mut self.inode_cur),
            None,
        );
        let name_node = NameNode::new("file1".into(), Node::File(file_node.hash.clone()));
        fake_root.add_file(&name_node);

        self.insert_inode(&INode::File(file_node));
        self.insert_inode(&INode::Tag(fake_root));
        self.insert_name_node(&name_node);

        Ok(())
    }

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

        // TODO: Still not proper block hashings

        let mut path = PathBuf::from(&self.data_dir);
        if let Ok(node) = self.get_inode(ino) {
            match node {
                INode::File(f) => {
                    path.push("filenodes");
                    path.push(f.hash.code.clone());
                }
                INode::Tag(_) => {
                    reply.error(EISDIR);
                    return;
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

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        debug!("readdir | ino: {}; offset: {}", ino, offset);

        if let Ok(INode::Tag(t)) = self.get_inode(ino) {
            let entries = t.dir_links;

            for (index, id) in entries.iter().skip(offset as usize).enumerate() {
                if let Ok(name_node) = self.get_name_node(&id) {
                    let (inode, file_type) = {
                        if let Ok(node) = self.get_node(&name_node.link) {
                            match node {
                                INode::File(f) => (f.file_attr.inode, f.file_attr.kind),
                                INode::Tag(t) => (t.dir_attr.inode, t.dir_attr.kind),
                            }
                        } else {
                            continue;
                        }
                    };
                    debug!("\t> {inode}, {file_type:?}, {:?}", name_node.name);

                    // i + 1 means the index of the next entry
                    // i-node, offset, type, name
                    let buffer_full: bool = reply.add(
                        inode,
                        offset + index as i64 + 1,
                        file_type.into(),
                        name_node.name,
                    );

                    if buffer_full {
                        break;
                    }
                }
            }

            reply.ok();
        } else {
            reply.error(ENOENT);
        }
    }

    fn create(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mut mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        debug!("create | parent: {parent}, name: {name:?}");
        let mut parent_attrs = match self.get_inode(parent) {
            Ok(attrs) => match attrs {
                INode::File(f) => f.file_attr,
                INode::Tag(t) => t.dir_attr,
            },
            Err(error_code) => {
                reply.error(error_code);
                return;
            }
        };

        // TODO: access checks
        parent_attrs.last_modified = time_now();
        parent_attrs.last_metadata_changed = time_now();

        if req.uid() != 0 {
            mode &= !(libc::S_ISUID | libc::S_ISGID) as u32;
        }

        let file_type = mode & libc::S_IFMT as u32;

        let file_type = match file_type {
            libc::S_IFREG => FileKind::File,
            libc::S_IFDIR => FileKind::Directory,
            _ => {
                reply.error(libc::ENOSYS);
                unimplemented!("mknod() implementation is incomplete. Only supports regular files and directories. Got {:o}", mode);
            }
        };

        let attrs = InodeAttributes {
            inode: 0,
            open_file_handles: 1,
            size: 0,
            last_accessed: time_now(),
            last_modified: time_now(),
            last_metadata_changed: time_now(),
            kind: file_type,
            mode: mode as u16,
            hardlinks: 1,
            uid: req.uid(),
            gid: req.gid(),
        };
        let mut inode = self.allocate_next_inode(file_type, Some(attrs));

        let parent_node = self.get_node_from_inode(parent).unwrap();
        if let INode::Tag(ref mut t) = inode {
            t.add_file(&NameNode::new(".".into(), Node::Tag(t.id)));
            t.add_file(&NameNode::new("..".into(), parent_node));
        };

        let mut parent_inode = self.get_inode(parent).unwrap();
        if let INode::Tag(ref mut t) = parent_inode {
            let name_node = NameNode::new(name.to_os_string(), inode.to_node());
            t.add_file(&name_node);
            self.insert_name_node(&name_node);
            self.insert_inode(&parent_inode);
        }

        // TODO: make it so after every modification inodes rewrite themselves?
        self.insert_inode(&inode);

        // TODO: implement flags
        match inode {
            INode::File(f) => {
                reply.created(
                    &Duration::new(0, 0),
                    &f.file_attr.into(),
                    0,
                    self.get_filehandle_cur(),
                    0,
                );
            }
            INode::Tag(t) => {
                reply.created(
                    &Duration::new(0, 0),
                    &t.dir_attr.into(),
                    0,
                    self.get_filehandle_cur(),
                    0,
                );
            }
        }
    }

    // TODO: refactor since create and mknod are basically doing the same thing except for the
    // filehandler?

    fn mknod(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mut mode: u32,
        _umask: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        debug!("mknod");
        //reply.error(ENOSYS);

        let file_type = mode & libc::S_IFMT as u32;

        let file_type = match file_type {
            libc::S_IFREG => FileKind::File,
            libc::S_IFDIR => FileKind::Directory,
            _ => {
                reply.error(libc::ENOSYS);
                unimplemented!("mknod() implementation is incomplete. Only supports regular files and directories. Got {:o}", mode);
            }
        };

        // We can't return EEXIST sort of - we can create an arbitrary number of files with the
        // same name, but different content and hash!

        let mut parent_attrs = match self.get_inode(parent) {
            Ok(attrs) => match attrs {
                INode::File(f) => f.file_attr,
                INode::Tag(t) => t.dir_attr,
            },
            Err(error_code) => {
                reply.error(error_code);
                return;
            }
        };

        // TODO: access checks
        parent_attrs.last_modified = time_now();
        parent_attrs.last_metadata_changed = time_now();

        if req.uid() != 0 {
            mode &= !(libc::S_ISUID | libc::S_ISGID) as u32;
        }

        let attrs = InodeAttributes {
            inode: 0,
            open_file_handles: 0,
            size: 0,
            last_accessed: time_now(),
            last_modified: time_now(),
            last_metadata_changed: time_now(),
            kind: file_type,
            mode: mode as u16,
            hardlinks: 1,
            uid: req.uid(),
            gid: req.gid(), // TODO: Proper uid, gid creation
        };
        let mut inode = self.allocate_next_inode(file_type, Some(attrs));

        let parent_node = self.get_node_from_inode(parent).unwrap();
        if let INode::Tag(ref mut t) = inode {
            t.add_file(&NameNode::new(".".into(), Node::Tag(t.id)));
            t.add_file(&NameNode::new("..".into(), parent_node));
        };

        let mut parent_inode = self.get_inode(parent).unwrap();
        if let INode::Tag(ref mut t) = parent_inode {
            let name_node = NameNode::new(name.to_os_string(), inode.to_node());
            t.add_file(&name_node);
            self.insert_name_node(&name_node);
            self.insert_inode(&parent_inode);
        }

        // TODO: make it so after every modification inodes rewrite themselves?
        self.insert_inode(&inode);

        // TODO: implement flags
        match inode {
            INode::File(f) => reply.entry(&Duration::new(0, 0), &f.file_attr.into(), 0),
            INode::Tag(t) => reply.entry(&Duration::new(0, 0), &t.dir_attr.into(), 0),
        }
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
        debug!("mkdir | unimplemented!");
        reply.error(ENOSYS);

        //if self.lookup_name(parent, name).is_ok() {
        //reply.error(libc::EEXIST);
        //return;
        //}

        //let mut parent_attrs = match self.get_inode(parent) {
        //Ok(attrs) => attrs,
        //Err(error_code) => {
        //reply.error(error_code);
        //return;
        //}
        //};

        //if !check_access(
        //parent_attrs.uid,
        //parent_attrs.gid,
        //parent_attrs.mode,
        //req.uid(),
        //req.gid(),
        //libc::W_OK,
        //) {
        //reply.error(libc::EACCES);
        //return;
        //}
        //parent_attrs.last_modified = time_now();
        //parent_attrs.last_metadata_changed = time_now();
        //self.write_inode(&parent_attrs);

        //if req.uid() != 0 {
        //mode &= !(libc::S_ISUID | libc::S_ISGID) as u32;
        //}
        //if parent_attrs.mode & libc::S_ISGID as u16 != 0 {
        //mode |= libc::S_ISGID as u32;
        //}

        //let inode = self.allocate_next_inode();
        //let attrs = InodeAttributes {
        //inode,
        //open_file_handles: 0,
        //size: BLOCK_SIZE,
        //last_accessed: time_now(),
        //last_modified: time_now(),
        //last_metadata_changed: time_now(),
        //kind: FileKind::Directory,
        //mode: self.creation_mode(mode),
        //hardlinks: 2, // Directories start with link count of 2, since they have a self link
        //uid: req.uid(),
        //gid: creation_gid(&parent_attrs, req.gid()),
        //xattrs: Default::default(),
        //};
        //self.write_inode(&attrs);

        //let mut entries = BTreeMap::new();
        //entries.insert(b".".to_vec(), (inode, FileKind::Directory));
        //entries.insert(b"..".to_vec(), (parent, FileKind::Directory));
        //self.write_directory_content(inode, entries);

        //let mut entries = self.get_directory_content(parent).unwrap();
        //entries.insert(name.as_bytes().to_vec(), (inode, FileKind::Directory));
        //self.write_directory_content(parent, entries);

        //reply.entry(&TTL, &attrs.into(), 0);
    }

    // NOTE: All the calls below this point are unimplemented, and return their default return
    // values, while also debug printing some information so we could use that while developing and
    // determining which functions need to be implemented for certain functionality to work
    //
    // TODO: Figure out what exactly is needed for simple functionality
    // As far as I can tell:
    //  * touch also calls setattr!
    //  * file attributes changing
    //  * "directory" creation
    //  * moving files and tags

    fn destroy(&mut self) {
        debug!("destroy | unimplemented!");
    }

    fn forget(&mut self, _req: &Request<'_>, _ino: u64, _nlookup: u64) {
        debug!("forget | unimplemented!");
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
        debug!("setattr | unimplemented!");
        reply.error(ENOSYS);
    }

    fn readlink(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyData) {
        debug!("readlink | unimplemented!");
        reply.error(ENOSYS);
    }

    fn unlink(&mut self, _req: &Request<'_>, _parent: u64, _name: &OsStr, reply: ReplyEmpty) {
        debug!("unlink | unimplemented!");
        reply.error(ENOSYS);
    }

    fn rmdir(&mut self, _req: &Request<'_>, _parent: u64, _name: &OsStr, reply: ReplyEmpty) {
        debug!("rmdir | unimplemented!");
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
        debug!("rmdir | unimplemented!");
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
        debug!("rename | unimplemented!");
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
        debug!("link | unimplemented!");
        reply.error(ENOSYS);
    }

    fn open(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        debug!("open | unimplemented!");
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
        debug!("write | unimplemented!");
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
        debug!("flush | unimplemented!");
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
        debug!("release | unimplemented!");
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
        debug!("fsync | unimplemented!");
        reply.error(ENOSYS);
    }

    fn opendir(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        debug!("opendir | unimplemented!");
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
        debug!("readdirplus | unimplemented!");
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
        debug!("releasedir | unimplemented!");
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
        debug!("fsyncdir | unimplemented!");
        reply.error(ENOSYS);
    }

    fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyStatfs) {
        debug!("statfs | unimplemented!");
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
        debug!("setxattr | unimplemented!");
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
        debug!("getxattr | unimplemented!");
        reply.error(ENOSYS);
    }

    fn listxattr(&mut self, _req: &Request<'_>, _ino: u64, _size: u32, reply: ReplyXattr) {
        debug!("listxattr | unimplemented!");
        reply.error(ENOSYS);
    }

    fn removexattr(&mut self, _req: &Request<'_>, _ino: u64, _name: &OsStr, reply: ReplyEmpty) {
        debug!("removexattr | unimplemented!");
        reply.error(ENOSYS);
    }

    fn access(&mut self, _req: &Request<'_>, _ino: u64, _mask: i32, reply: ReplyEmpty) {
        debug!("access | unimplemented!");
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
        debug!("getlk | unimplemented!");
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
        debug!("setlk | unimplemented!");
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
        debug!("bmap | unimplemented!");
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
        debug!("ioctl | unimplemented!");
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
        debug!("fallocate | unimplemented!");
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
        debug!("lseek | unimplemented!");
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
        debug!("copy_file_range | unimplemented!");
        reply.error(ENOSYS);
    }
}
