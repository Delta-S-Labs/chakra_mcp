-- Owner consent, PR 1: validate the CHECKs 0041 and 0042 added NOT VALID.
-- Design: docs/superpowers/specs/2026-10-23-owner-consent-design.md §9.
--
-- In a file of its own because sqlx runs each migration in one transaction:
-- validating in 0041 or 0042 would scan while still holding their ACCESS
-- EXCLUSIVE lock. VALIDATE CONSTRAINT takes only SHARE UPDATE EXCLUSIVE, which
-- doesn't conflict with reads or writes, so invocations continue during the
-- scan.

ALTER TABLE agent_capabilities VALIDATE CONSTRAINT agent_capabilities_consent_max_validity_check;
ALTER TABLE agent_capabilities VALIDATE CONSTRAINT agent_capabilities_consent_wait_check;
ALTER TABLE agent_capabilities VALIDATE CONSTRAINT agent_capabilities_consent_approvers_check;
ALTER TABLE relay_invocations VALIDATE CONSTRAINT relay_invocations_status_check;
