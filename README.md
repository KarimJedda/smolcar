# Smolcar

Smolcar! It's like smoldot and sidecar had a baby.

A lightweight Substrate/Polkadot blockchain indexer that uses smoldot-light for syncing and SQLite for persistent storage. No full node required.


## What does it do?

Connects to any Substrate-based chain via a light client, indexes finalized blocks with all their extrinsics and events, stores everything in SQLite, and exposes a simple REST API to query blocks.

## Why?

I'll run this as WASM in the browser too, because I can / want. But also sometimes you want to index a chain without running a full node or archive node, or you want to do it in the browser 🤷. Smoldot gives you a light client, but you still need to store and query the data. Smolcar does that.

## Quick Start

```bash
cargo run
```

The application will:
- Connect to Polkadot via smoldot light client
- Create `blocks.db` in the current directory
- Start indexing finalized blocks
- Expose API on http://localhost:8080

## API

**Get latest block:**
```bash
curl http://localhost:8080/blocks/head
```

**Get specific block:**
```bash
curl http://localhost:8080/block/23456789
```

Note: this assumes you already fetched this block. You can get an sqlite from a friend too and it'll work. A provision to verify the sqlite dbs will be implemented later so we can do this trustlessly. 

## Configuration

Edit `src/main.rs` to configure filtering:

**Exclude noisy events:**
```rust
const EXCLUDED_EVENTS: &[(&str, Option<&str>)] = &[
    ("System", Some("ExtrinsicSuccess")),
    ("ParaInclusion", None),
];
```

**Exclude noisy extrinsics:**
```rust
const EXCLUDED_EXTRINSICS: &[&str] = &[
    "Timestamp/set",
    "ParaInherent/enter",
];
```

**Change chain:**
Replace `polkadot.json` with any chain spec and update `POLKADOT_SPEC` constant.

## Data Structure

Blocks are stored with events nested inside extrinsics, showing exactly which events each extrinsic emitted:

```json
{
  "number": 23456789,
  "hash": "0x...",
  "extrinsics": [
    {
      "index": 0,
      "hash": "0xabc...",
      "action": "Balances/transfer",
      "params": "dest: 5Grw..., value: 1000000000000",
      "events": [
        {"pallet": "Balances", "variant": "Transfer", "data": "..."},
        {"pallet": "System", "variant": "ExtrinsicSuccess", "data": "..."}
      ]
    }
  ]
}
```


## Requirements

- Rust 1.70+
- About 10MB RAM per chain (smoldot is tiny)
- Disk space for the SQLite database (grows with indexed blocks)
