-- Initial schema setup for Federation Tools OSS
-- Creates base tables and types

-- Create custom types
CREATE TYPE subscription_status AS ENUM (
    'pending_initial_activation',
    'active',
    'expired',
    'cancelled'
);

CREATE TYPE guardian_role AS ENUM (
    'lead_guardian',
    'other_guardian'
);

CREATE TYPE federation_status AS ENUM (
    'pending',
    'configuring',
    'running_dkg',
    'active',
    'failed'
);

-- Create extension for UUID generation if not exists
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- Base system metadata table
CREATE TABLE IF NOT EXISTS system_metadata (
    id SERIAL PRIMARY KEY,
    key VARCHAR(255) UNIQUE NOT NULL,
    value TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Insert version information
INSERT INTO system_metadata (key, value)
VALUES ('schema_version', '1.0.0')
ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = CURRENT_TIMESTAMP;

-- Create updated_at trigger function
CREATE OR REPLACE FUNCTION update_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = CURRENT_TIMESTAMP;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Create indexes for common queries
CREATE INDEX IF NOT EXISTS idx_system_metadata_key ON system_metadata(key);