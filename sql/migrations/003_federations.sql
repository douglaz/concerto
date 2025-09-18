-- Federation and guardian management tables

-- Federations table
CREATE TABLE IF NOT EXISTS federations (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    federation_id VARCHAR(255) UNIQUE NOT NULL,
    name VARCHAR(255) NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    subscription_id UUID REFERENCES subscriptions(id),
    status federation_status NOT NULL DEFAULT 'pending',
    num_guardians INTEGER NOT NULL,
    invite_code TEXT,
    config_hash VARCHAR(64),
    kubernetes_namespace VARCHAR(255),
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Guardians table
CREATE TABLE IF NOT EXISTS guardians (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    federation_id UUID NOT NULL REFERENCES federations(id) ON DELETE CASCADE,
    guardian_index INTEGER NOT NULL,
    role guardian_role NOT NULL,
    nostr_npub VARCHAR(255) NOT NULL,
    peer_id VARCHAR(255),
    api_url TEXT,
    p2p_url TEXT,
    status VARCHAR(50) DEFAULT 'pending',
    kubernetes_pod_name VARCHAR(255),
    last_health_check TIMESTAMP WITH TIME ZONE,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(federation_id, guardian_index)
);

-- Federation events/audit log
CREATE TABLE IF NOT EXISTS federation_events (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    federation_id UUID NOT NULL REFERENCES federations(id) ON DELETE CASCADE,
    event_type VARCHAR(100) NOT NULL,
    event_data JSONB NOT NULL,
    user_id VARCHAR(255),
    guardian_id UUID REFERENCES guardians(id),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Guardian registry for OG (Other Guardian) coordination
CREATE TABLE IF NOT EXISTS og_registry (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    nostr_npub VARCHAR(255) UNIQUE NOT NULL,
    guardian_name VARCHAR(255),
    api_endpoint TEXT,
    is_available BOOLEAN DEFAULT true,
    reputation_score INTEGER DEFAULT 0,
    total_federations INTEGER DEFAULT 0,
    active_federations INTEGER DEFAULT 0,
    metadata JSONB DEFAULT '{}',
    last_seen_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Federation launcher configuration
CREATE TABLE IF NOT EXISTS federation_launcher_config (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    federation_id UUID NOT NULL REFERENCES federations(id) ON DELETE CASCADE,
    kubernetes_config JSONB NOT NULL,
    deployment_status VARCHAR(50) DEFAULT 'pending',
    deployment_error TEXT,
    deployed_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Create indexes
CREATE INDEX IF NOT EXISTS idx_federations_user_id ON federations(user_id);
CREATE INDEX IF NOT EXISTS idx_federations_status ON federations(status);
CREATE INDEX IF NOT EXISTS idx_federations_federation_id ON federations(federation_id);
CREATE INDEX IF NOT EXISTS idx_guardians_federation_id ON guardians(federation_id);
CREATE INDEX IF NOT EXISTS idx_guardians_nostr_npub ON guardians(nostr_npub);
CREATE INDEX IF NOT EXISTS idx_guardians_status ON guardians(status);
CREATE INDEX IF NOT EXISTS idx_federation_events_federation_id ON federation_events(federation_id);
CREATE INDEX IF NOT EXISTS idx_federation_events_created_at ON federation_events(created_at);
CREATE INDEX IF NOT EXISTS idx_og_registry_nostr_npub ON og_registry(nostr_npub);
CREATE INDEX IF NOT EXISTS idx_og_registry_is_available ON og_registry(is_available);

-- Add triggers for updated_at
CREATE TRIGGER update_federations_updated_at
    BEFORE UPDATE ON federations
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();

CREATE TRIGGER update_guardians_updated_at
    BEFORE UPDATE ON guardians
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();

CREATE TRIGGER update_og_registry_updated_at
    BEFORE UPDATE ON og_registry
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();

CREATE TRIGGER update_federation_launcher_config_updated_at
    BEFORE UPDATE ON federation_launcher_config
    FOR EACH ROW EXECUTE FUNCTION update_updated_at();