# tag_fs - A simple tag fileystem

## Installation

Make sure you have FUSE and libfuse installed, they're usually available in a package
called `fuse` in your default package manager. Install Rust too!

```
git clone https://github.com/last-genius/tag_fs
cargo build
```

## Usage

```
sudo target/debug/tag_fs $MOUNT_POINT
```
