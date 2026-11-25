# SaveSync Agents

## Overview
SaveSync uses a multi-agent architecture where each component is implemented as a separate crate in the Rust workspace. The system consists of four main agents that work together to provide cross-platform save synchronization.

## Agent Structure

### `daemon` - Core Sync Engine
- **Purpose**: Main daemon responsible for all sync logic, watchers, conflict resolution, and backend operations
- **Responsibilities**:
  - File watching (notify crate)
  - Snapshot creation and management
  - Conflict detection and resolution
  - Local index management
  - Backend communication (Yandex Disk, Google Drive, LAN)
- **Interface**: Unix socket or named pipe for IPC with UI agents

### `app` - Desktop Application
- **Purpose**: Cross-platform desktop GUI for Windows and SteamOS
- **Responsibilities**:
  - User interface for game management
  - Sync control (manual trigger)
  - Configuration management
  - Backup operations
- **Interface**: Communicates with daemon via JSON-RPC over IPC

### `decky-bridge` - Decky Plugin Bridge
- **Purpose**: Thin bridge between Decky Loader plugin and daemon
- **Responsibilities**:
  - Provides quick access status in Steam Deck Quick Access Menu
  - Sync trigger button
  - Conflict notification banner
- **Interface**: Unix socket communication with daemon

### `core` - Shared Logic Library
- **Purpose**: Core logic shared across all agents
- **Responsibilities**:
  - Game identification and slug resolution
  - Manifest parsing and management
  - Path resolution for different platforms
  - Hashing (Blake3)
  - Scanner logic for finding games on filesystem

## Agent Interactions
```
┌─────────────┐    IPC    ┌─────────────┐    IPC    ┌─────────────┐
│   Desktop   │ ◀───────▶ │   Daemon    │ ◀───────▶ │ Decky Plugin│
│   App       │           │             │           │ Bridge      │
└─────────────┘           └─────────────┘           └─────────────┘
                                 ▲
                                 │
                            ┌────┴────┐
                            │  Core   │
                            └─────────┘
```

## Implementation Status (M1)
- Core crate: Basic manifest parsing, scanner, path resolution
- Daemon crate: Skeleton with placeholder for sync engine implementation
- Desktop app crate: Empty skeleton
- Decky bridge crate: Empty skeleton

## Future Development Plan
1. **M1**: Implement core logic and basic daemon functionality
2. **M2**: Add cloud backend support (Yandex Disk/Google Drive)
3. **M3**: Implement LAN sync capabilities with mDNS discovery
4. **M4**: Complete desktop application UI
5. **M5**: Finish Decky plugin implementation