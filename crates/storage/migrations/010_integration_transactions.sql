-- Legacy rows have no proof of ownership of a particular configuration entry.
ALTER TABLE client_integrations ADD COLUMN scope TEXT NOT NULL DEFAULT 'PROJECT';
ALTER TABLE client_integrations ADD COLUMN original_hash TEXT;
ALTER TABLE client_integrations ADD COLUMN published_hash TEXT;
ALTER TABLE client_integrations ADD COLUMN pending TEXT CHECK(pending IN ('INSTALL','REMOVE'));
ALTER TABLE client_integrations ADD COLUMN removal_hash TEXT;
ALTER TABLE client_integrations ADD COLUMN verification TEXT CHECK(verification IN ('GATEWAY','CLIENT'));
ALTER TABLE client_integrations ADD COLUMN source_hash TEXT;
