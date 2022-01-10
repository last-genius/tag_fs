use fuser::{FileAttr, FileType};
use libc::{getgid, getuid};
use serde::{ser::SerializeStruct, Deserialize, Serialize, Serializer};
use sha3::{
    digest::{generic_array::GenericArray, FixedOutput},
    Digest, Sha3_256,
};
//use std::collections::BTreeMap;
use std::{
    fmt::Display,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

// These two constants are just temporary, TODO
pub const TTL: Duration = Duration::from_secs(1);
pub const HELLO_TXT_CONTENT: &str = "Hello World!\n";

const BLOCK_SIZE: u64 = 512;

// Helper time functions section
fn time_now() -> (i64, u32) {
    time_from_system_time(&SystemTime::now())
}

fn system_time_from_time(secs: i64, nsecs: u32) -> SystemTime {
    if secs >= 0 {
        UNIX_EPOCH + Duration::new(secs as u64, nsecs)
    } else {
        UNIX_EPOCH - Duration::new((-secs) as u64, nsecs)
    }
}

fn time_from_system_time(system_time: &SystemTime) -> (i64, u32) {
    // Convert to signed 64-bit time with epoch at 0
    match system_time.duration_since(UNIX_EPOCH) {
        Ok(duration) => (duration.as_secs() as i64, duration.subsec_nanos()),
        Err(before_epoch_error) => (
            -(before_epoch_error.duration().as_secs() as i64),
            before_epoch_error.duration().subsec_nanos(),
        ),
    }
}

// Hash section
type HashArray = GenericArray<u8, <Sha3_256 as FixedOutput>::OutputSize>;

#[derive(PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub struct Hash256 {
    code: HashArray,
}

pub trait HashCalculate {
    fn calculate_hash(self: Self) -> Hash256;
}

impl HashCalculate for Sha3_256 {
    fn calculate_hash(self: Self) -> Hash256 {
        Hash256 {
            code: self.finalize_reset(),
        }
    }
}

impl Display for Hash256 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:x}", self.code)
    }
}
impl Serialize for Hash256 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // 3 is the number of fields in the struct.
        let mut state = serializer.serialize_struct("Hash256", 1)?;
        state.serialize_field("code", &format!("{:x}", &self.code))?;
        state.end()
    }
}
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};

impl<'de> Deserialize<'de> for Hash256 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Secs,
            Nanos,
        }

        struct DurationVisitor;

        impl<'de> Visitor<'de> for DurationVisitor {
            type Value = Duration;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct Hash256")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Duration, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let secs = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let nanos = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(Duration::new(secs, nanos))
            }

            fn visit_map<V>(self, mut map: V) -> Result<Duration, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut secs = None;
                let mut nanos = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Secs => {
                            if secs.is_some() {
                                return Err(de::Error::duplicate_field("secs"));
                            }
                            secs = Some(map.next_value()?);
                        }
                        Field::Nanos => {
                            if nanos.is_some() {
                                return Err(de::Error::duplicate_field("nanos"));
                            }
                            nanos = Some(map.next_value()?);
                        }
                    }
                }
                let secs = secs.ok_or_else(|| de::Error::missing_field("secs"))?;
                let nanos = nanos.ok_or_else(|| de::Error::missing_field("nanos"))?;
                Ok(Duration::new(secs, nanos))
            }
        }

        const FIELDS: &'static [&'static str] = &["secs", "nanos"];
        deserializer.deserialize_struct("Duration", FIELDS, DurationVisitor)
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, PartialEq)]
pub enum FileKind {
    File,
    Directory,
    Symlink,
}

impl From<FileKind> for fuser::FileType {
    fn from(kind: FileKind) -> Self {
        match kind {
            FileKind::File => fuser::FileType::RegularFile,
            FileKind::Directory => fuser::FileType::Directory,
            FileKind::Symlink => fuser::FileType::Symlink,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct InodeAttributes {
    pub inode: u64,
    pub open_file_handles: u64, // Ref count of open file handles to this inode
    pub size: u64,
    pub last_accessed: (i64, u32),
    pub last_modified: (i64, u32),
    pub last_metadata_changed: (i64, u32),
    pub kind: FileKind,
    pub mode: u16,
    pub hardlinks: u32,
    pub uid: u32,
    pub gid: u32,
    //pub xattrs: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl From<InodeAttributes> for fuser::FileAttr {
    fn from(attrs: InodeAttributes) -> Self {
        fuser::FileAttr {
            ino: attrs.inode,
            size: attrs.size,
            blocks: (attrs.size + BLOCK_SIZE - 1) / BLOCK_SIZE,
            atime: system_time_from_time(attrs.last_accessed.0, attrs.last_accessed.1),
            mtime: system_time_from_time(attrs.last_modified.0, attrs.last_modified.1),
            ctime: system_time_from_time(
                attrs.last_metadata_changed.0,
                attrs.last_metadata_changed.1,
            ),
            crtime: SystemTime::UNIX_EPOCH,
            kind: attrs.kind.into(),
            perm: attrs.mode,
            nlink: attrs.hardlinks,
            uid: attrs.uid,
            gid: attrs.gid,
            rdev: 0,
            blksize: BLOCK_SIZE as u32,
            flags: 0,
        }
    }
}

impl InodeAttributes {
    pub fn new_file_attr(inode: u64, kind: FileKind, mode: u16) -> Self {
        Self {
            inode,
            open_file_handles: 0,
            size: 0,
            last_accessed: time_now(),
            last_modified: time_now(),
            last_metadata_changed: time_now(),
            kind,
            mode,
            hardlinks: 0,
            uid: unsafe { getuid() },
            gid: unsafe { getgid() },
        }
    }
}

// TODO this is temporary as well
lazy_static::lazy_static! {
    static ref FAKE_ROOT_DIR_ATTR: InodeAttributes = InodeAttributes::new_file_attr(1, FileKind::Directory, 0x755);

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
