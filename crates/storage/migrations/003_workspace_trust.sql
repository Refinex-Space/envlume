-- T02/T03 fixture registrations have no proof of interactive approval. They
-- remain inspectable, but only a new confirmation can authorize a production binding.
ALTER TABLE workspace_bindings ADD COLUMN entry_path TEXT CHECK(length(entry_path) BETWEEN 1 AND 32767);
ALTER TABLE workspace_bindings ADD COLUMN root_identity TEXT CHECK(length(root_identity) BETWEEN 1 AND 256);
ALTER TABLE workspace_bindings ADD COLUMN confirmation_source TEXT CHECK(confirmation_source = 'LOCAL_CLI_TTY');
ALTER TABLE workspace_bindings ADD COLUMN confirmed_at_ms INTEGER CHECK(confirmed_at_ms >= 0);
DROP INDEX active_workspace_path;
CREATE UNIQUE INDEX active_workspace_path ON workspace_bindings(canonical_path) WHERE trust_state IN ('TRUSTED','UNTRUSTED');
CREATE UNIQUE INDEX active_workspace_entry ON workspace_bindings(entry_path) WHERE trust_state='TRUSTED';
CREATE UNIQUE INDEX active_workspace_identity ON workspace_bindings(root_identity) WHERE trust_state='TRUSTED';
