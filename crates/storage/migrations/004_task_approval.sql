-- Approval belongs to a particular binding's live run grant, not the shared
-- project template. Earlier grants must undergo first-use confirmation.
ALTER TABLE capability_grants ADD COLUMN timeout_seconds INTEGER CHECK(timeout_seconds BETWEEN 1 AND 3600);
ALTER TABLE capability_grants ADD COLUMN approval_digest TEXT CHECK(length(approval_digest)=64);
ALTER TABLE capability_grants ADD COLUMN approved_at_ms INTEGER CHECK(approved_at_ms >= 0);
ALTER TABLE capability_grants ADD COLUMN distribution_digest TEXT CHECK(length(distribution_digest)=64);
