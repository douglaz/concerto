# Concerto Architecture Documentation

## System Overview

Concerto is a greenfield implementation of a distributed, Nostr-coordinated federation management system for Fedimint. It eliminates central authorities by using Nostr for all coordination, creating a truly decentralized ecosystem.

## Core Design Principles

1. **No Central Authority**: All coordination happens via Nostr protocol
2. **Economic Independence**: FeLaaS providers operate as independent businesses
3. **Guardian Sovereignty**: Each guardian bot only responds to its owner
4. **Open Market**: Multiple providers compete on price and features
5. **Flexible Resources**: Slot-based allocation model
6. **Transparent Coordination**: All activity visible on Nostr

## Component Architecture

### 1. Concerto-Common (Shared Library)

The foundation layer providing data models and types used across all components:

#### Subscription System (`subscription.rs`)
- **Subscription Management**: Tracks owner subscriptions with payment info
- **Subscription Plans**: Tiered plans (Basic, Professional, Enterprise, Custom)
- **Subscription Proofs**: Cryptographic proofs of slot ownership
- **Payment Tracking**: Invoice generation and payment status

#### Federation Models (`federation.rs`)
- **Federation Lifecycle**: Proposed → Forming → ConfiguringDKG → Active
- **Guardian Management**: Lead Guardian and Other Guardian roles
- **Requirements System**: Min slots, geographic diversity, feature requirements
- **Consensus Configuration**: Threshold calculations and parameters

#### Slot Management (`slot.rs`)
- **Slot States**: Available → Allocated → Launching → Running → Stopped
- **Health Monitoring**: Healthy, Unhealthy, Degraded states
- **Resource Tracking**: CPU, memory, storage, bandwidth usage
- **Allocation System**: Provider assignment and endpoint management

#### Economic Models (`economics.rs`)
- **Dynamic Pricing**: Base price + surge pricing + volume discounts
- **Risk Assessment**: Federation risk scoring and recommendations
- **Revenue Tracking**: Billing records and revenue reports
- **Provider Economics**: Profit margins and utilization rates

#### Nostr Events (`nostr_events.rs`)
- **Custom Event Kinds**: 30500-30504 for federation coordination
- **Event Types**:
  - Federation Proposal (30500)
  - Guardian Application (30501)
  - Application Decision (30502)
  - Slot Allocation (30503)
  - DKG Coordination (30504)
  - Service Advertisement (30600)

### 2. Concerto-Guardianito (Guardian Tool)

The guardian client that manages federations on behalf of its owner:

#### Core Components

**Guardianito Core (`guardianito.rs`)**
- Owner-only authentication via npub
- Subscription management
- Federation lifecycle management
- Slot allocation coordination

**Nostr Client (`nostr_client.rs`)**
- Event publishing and subscription
- Encrypted DM handling
- Relay management
- Query operations

**State Management (`state.rs`)**
- Persistent storage with sled database
- Federation state tracking
- Subscription caching
- Event aggregation

**Command Processing (`commands.rs`)**
- CLI interface for local commands
- Nostr DM command processing
- Subscription commands
- Federation management commands

#### Protocol Implementations

**DKG Coordination (`dkg.rs`)**
- Multi-round DKG protocol
- Commitment phase
- Share distribution
- Verification and finalization
- Threshold calculations

**Configuration Exchange (`config_exchange.rs`)**
- Peer-to-peer config sharing
- Fedimint config generation
- Consistency verification
- Encrypted config transport

**Federation Activation (`activation.rs`)**
- Step-by-step activation flow
- Guardian gathering
- Slot allocation
- Config exchange
- DKG execution
- Health verification

**Service Discovery (`discovery.rs`)**
- Provider discovery
- Service filtering and ranking
- Reputation scoring
- Real-time monitoring
- Health checks

### 3. Concerto-FeLaaS (Provider System)

Economically independent infrastructure providers:

#### Provider Core (`provider.rs`)
- **Economic Validation**: Subscription verification
- **Dynamic Pricing**: Demand-based price adjustments
- **Resource Management**: Slot allocation and tracking
- **Risk Assessment**: Federation evaluation

#### API Layer (`api.rs`)
- RESTful API for management
- Provider info endpoints
- Slot allocation endpoints
- Pricing queries
- Utilization metrics

#### Deployment Backends (`deployment.rs`)
- **Docker Backend**: Container-based deployment
- **Kubernetes Backend**: Orchestrated deployment
- Resource limits and monitoring
- Health check integration

#### Database Layer (`database.rs`)
- PostgreSQL schema
- Subscription validation tracking
- Hosted slots management
- Economic metrics storage
- Invoice and billing records

## Data Flow Architecture

### Federation Formation Flow

```
1. Initiator proposes federation via Nostr event
2. Guardians discover proposal through service discovery
3. Guardians apply with subscription proofs
4. Initiator approves/rejects applications
5. Approved guardians allocate slots from providers
6. Configuration exchange protocol runs
7. DKG protocol generates federation keys
8. Federation activates and goes live
```

### Economic Flow

```
1. Guardian purchases subscription from provider
2. Provider issues cryptographic proof
3. Guardian uses proof to allocate slots
4. Provider verifies proof and allocates resources
5. Provider tracks usage and generates billing
6. Revenue optimization adjusts pricing dynamically
```

### Nostr Coordination Flow

```
1. All events published to multiple relays
2. Participants subscribe to relevant event types
3. Encrypted DMs for private coordination
4. Event aggregation reconstructs state
5. Cryptographic signatures ensure authenticity
```

## Security Architecture

### Authentication & Authorization
- **Owner Authentication**: Guardianito only responds to owner's npub
- **Subscription Proofs**: Cryptographic proof of slot ownership
- **Nostr Signatures**: All events cryptographically signed
- **Encrypted Communications**: NIP-04 encrypted DMs

### Trust Model
- **No Central Authority**: Trust distributed across participants
- **Economic Incentives**: Providers motivated by revenue
- **Reputation System**: Service discovery includes reputation
- **Transparent Operations**: All coordination visible on Nostr

### Cryptographic Components
- **DKG Protocol**: Distributed key generation for federations
- **Signature Verification**: Event and proof validation
- **Encrypted Configs**: Secure configuration exchange
- **Payment Proofs**: Lightning invoice verification

## Scalability Considerations

### Horizontal Scaling
- Multiple FeLaaS providers compete
- Guardians can use any compatible provider
- Federations operate independently
- Nostr relays provide redundancy

### Economic Scaling
- Dynamic pricing responds to demand
- Volume discounts encourage growth
- Risk assessment prevents overcommitment
- Resource tracking enables optimization

### Technical Scaling
- Slot-based resource allocation
- Docker/Kubernetes deployment options
- Event aggregation for state reconstruction
- Async processing throughout

## Testing Strategy

### Unit Tests (Implemented)
- 46+ test cases across all components
- Subscription system validation
- Federation lifecycle testing
- Slot management verification
- Economic model validation
- Nostr event serialization

### Integration Tests (Planned)
- End-to-end federation formation
- Multi-guardian coordination
- Provider interaction testing
- Nostr relay integration

### Load Tests (Planned)
- 100+ federation support
- Concurrent guardian operations
- Provider capacity limits
- Nostr event throughput

## Deployment Architecture

### Development Environment
- Nix flakes for reproducible builds
- Cargo workspace for modular development
- Local sled database for guardianito
- PostgreSQL for FeLaaS providers

### Production Environment (Planned)
- Containerized deployment
- Kubernetes orchestration
- PostgreSQL with migrations
- Monitoring and alerting
- Backup and recovery

## Future Enhancements

### Short Term
1. Complete nostr-sdk API compatibility
2. Implement proper cryptographic functions
3. Add monitoring and metrics
4. Create deployment manifests

### Medium Term
1. Lightning integration for payments
2. Advanced reputation system
3. Automated testing suite
4. Performance optimizations

### Long Term
1. Multi-federation management
2. Cross-federation interactions
3. Advanced economic models
4. Decentralized governance

## Conclusion

Concerto represents a complete reimagining of federation management, removing all central points of control and creating a truly distributed system. By leveraging Nostr for coordination and implementing strong economic incentives, it creates a sustainable ecosystem where providers compete to offer the best service while guardians maintain complete sovereignty over their operations.