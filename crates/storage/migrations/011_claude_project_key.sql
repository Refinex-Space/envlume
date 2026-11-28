-- Exact project key observed from the client, scoped by the existing binding and profile location.
ALTER TABLE client_integrations ADD COLUMN client_project_key TEXT;
