# Concerto - Distributed Federation Management via Nostr

Concerto is a greenfield implementation of a distributed, Nostr-coordinated federation management system for Fedimint. It eliminates central authorities by using Nostr for all coordination, creating a truly decentralized ecosystem.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Nostr Network                        │
│  (Federation Coordination, Discovery, Communication)    │
└─────────────────────────────────────────────────────────┘
        ↑                    ↑                    ↑
        │                    │                    │
┌───────┴───────┐    ┌───────┴───────┐    ┌───────┴───────┐
│  Guardianito  │    │  Guardianito  │    │  Guardianito  │
│   (Owner 1)   │    │   (Owner 2)   │    │   (Owner 3)   │
└───────┬───────┘    └───────┬───────┘    └───────┬───────┘
        │                    │                    │
        ↓                    ↓                    ↓
┌───────────────────────────────────────────────────────┐
│              Multiple FeLaaS Providers                 │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐           │
│  │Provider A│  │Provider B│  │Provider C│  ...       │
│  └──────────┘  └──────────┘  └──────────┘           │
└───────────────────────────────────────────────────────┘
```

## Components

### 1. Guardianito - Universal Guardian Tool
A CLI tool that manages federations on behalf of its owner.

**Key Features:**
- **Owner-only authentication**: Only responds to commands from the owner's Nostr npub
- **Federation management**: Propose, join, and manage federations
- **Slot allocation**: Manage fedimint slots from subscriptions
- **Nostr native**: All coordination through Nostr events

### 2. FeLaaS - Federation-as-a-Service Provider
Economically independent infrastructure providers that host federation slots.

**Key Features:**
- **Economic model**: Dynamic pricing, subscription verification, billing
- **Resource management**: Track CPU, memory, storage, bandwidth usage
- **Risk assessment**: Evaluate federations before accepting
- **Multi-backend**: Support for Docker and Kubernetes

### 3. Common Library
Shared data models and types used across components.

## Quick Start

### Prerequisites
```bash
# For NixOS/Nix users
nix-shell -p pkg-config openssl

# For other systems, install:
# - pkg-config
# - OpenSSL development libraries
# - PostgreSQL (for FeLaaS)
```

### Building
```bash
cd concerto
cargo build --release
```

### Running Guardianito
```bash
# Generate or use existing Nostr keys
export GUARDIAN_NSEC="your-nsec-key"
export OWNER_NPUB="your-owner-npub"

# Run in daemon mode
./target/release/guardianito \
  --owner-npub $OWNER_NPUB \
  --guardian-nsec $GUARDIAN_NSEC \
  --relays "wss://relay.damus.io,wss://relay.nostr.band" \
  daemon
```

### Running FeLaaS Provider
```bash
# Set up PostgreSQL database
export DATABASE_URL="postgres://user:pass@localhost/felaas"

# Initialize database
./target/release/felaas init-db

# Run provider daemon
./target/release/felaas \
  --provider-nsec "provider-nsec-key" \
  --provider-name "My FeLaaS Provider" \
  --base-slot-price-sats 100000 \
  daemon
```

## Usage Examples

### Proposing a Federation
```bash
guardianito federation propose \
  --name "My Federation" \
  --my-slots 2 \
  --total-slots 4
```

### Applying to Join a Federation
```bash
guardianito federation apply \
  --federation-id "fed_abc123" \
  --slots 1 \
  --message "I'd like to join!"
```

### Approving a Guardian (Initiator Only)
```bash
guardianito federation approve \
  --federation-id "fed_abc123" \
  --guardian-npub "npub1..."
```

## Nostr Event Types

Concerto uses custom Nostr event kinds for coordination:

- **30500**: Federation Proposal
- **30501**: Guardian Application
- **30502**: Application Decision
- **30503**: Slot Allocation
- **30504**: DKG Coordination
- **30600**: Service Advertisement

## Economic Model

### Subscriptions
- Guardians purchase subscription packages containing fedimint slots
- Slots can be flexibly allocated to different federations
- Subscriptions are verified cryptographically by providers

### Provider Economics
- Dynamic pricing based on utilization
- Resource-based billing (CPU, memory, storage, bandwidth)
- Risk-adjusted pricing for different federations
- Volume discounts for larger subscriptions

### Market Dynamics
- Multiple providers compete on price, features, and reliability
- Guardians can choose providers based on their needs
- Transparent pricing advertised via Nostr
- No central authority controls the market

## Security Considerations

1. **Owner Authentication**: Guardianito only accepts commands from the configured owner npub
2. **Subscription Proofs**: Cryptographic proof of slot ownership
3. **Nostr Signatures**: All events are cryptographically signed
4. **Encrypted DMs**: Guardian commands sent via encrypted Nostr DMs
5. **Provider Verification**: Providers verify subscription validity before allocation

## Development Status

This is a greenfield implementation demonstrating the NEW_VISION architecture. Current status:
- ✅ Core data models
- ✅ Guardianito CLI with owner auth
- ✅ FeLaaS provider with economic model
- ✅ Nostr event types
- ✅ Basic slot management
- 🚧 DKG coordination
- 🚧 Production database migrations
- 🚧 Comprehensive testing

See [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md) for detailed progress.

## Contributing

This is an open-source project. Contributions are welcome!

## License

MIT License - See LICENSE file for details