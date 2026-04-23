-- Track the last known version for each provider YAML.
-- Used at startup to detect version bumps and trigger automatic re-syncs.
CREATE TABLE ProviderVersion (
    provider_name TEXT NOT NULL PRIMARY KEY,
    version       TEXT NOT NULL,
    updated_at    INTEGER NOT NULL
);
