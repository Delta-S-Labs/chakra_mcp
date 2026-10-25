-- Calls, C1: validate the CHECK 0045 added NOT VALID.
-- Design: docs/superpowers/specs/2026-10-25-calls-design.md.
--
-- In a file of its own, as 0043: VALIDATE CONSTRAINT takes only SHARE
-- UPDATE EXCLUSIVE, which doesn't conflict with reads or writes.

ALTER TABLE agents VALIDATE CONSTRAINT agents_accept_calls_check;
