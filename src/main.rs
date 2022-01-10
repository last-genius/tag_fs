#![feature(map_first_last)]
use clap::{crate_version, App, Arg};
use fuser::MountOption;

mod fs;

fn main() {
    let matches = App::new("tag_fs")
        .version(crate_version!())
        .arg(
            Arg::with_name("MOUNT_POINT")
                .required(true)
                .index(1)
                .help("Act as a client, and mount FUSE at given path"),
        )
        .get_matches();
    env_logger::init();

    let mountpoint = matches.value_of("MOUNT_POINT").unwrap();
    // TODO: In the future, switch to RW filesystem, choose sync or async i/o, allow execution of
    // binaries
    let options = vec![
        MountOption::RW,
        MountOption::FSName("tag_fs".to_string()),
        MountOption::AutoUnmount,
        MountOption::AllowOther,
    ];
    let fs = fs::TagFS::new();
    use std::fs::File;
    use std::io::prelude::*;
    let mut file = File::open("/mnt/tagfs/foo.txt").unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    println!("File: {}", contents);
    fuser::mount2(fs, mountpoint, &options).unwrap();
}
