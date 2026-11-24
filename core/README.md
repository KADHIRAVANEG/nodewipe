# nodewipe-core

The core scanning, block calculation, and excision engine powering [nodewipe](https://github.com/KADHIRAVANEG/nodewipe).

## Overview
- **Parallel Directory Traversal**: High-speed multithreaded directory scanning with Rayon.
- **Physical Block Accounting**: Accurate cluster block disk usage calculation using `st_blocks * 512`.
- **39+ Dev Ecosystems**: Detection for Node, Rust, Python, Go, Java, .NET, C/C++, AI/ML caches, mobile build outputs, and more.
- **Copy-on-Write Excision**: Instant `reflink` snapshots on Btrfs/XFS/APFS with automatic `.tar.gz` and OS Trash fallback.

## Usage
Add to your `Cargo.toml`:
```toml
[dependencies]
nodewipe-core = "1.0.0"
```

For full documentation, visit [github.com/KADHIRAVANEG/nodewipe](https://github.com/KADHIRAVANEG/nodewipe).
